import { EngineError, setTransport, type IpcTransport, type Unlisten } from "../ipc";
import { suggestedFileName } from "../exportName";
import type {
  AccountView, ActivityEntry, ActivityError, BoundSignIn, ConnectionReport, ConnectionView, DashboardRow, DashboardView, EntityCounts, EntityDetails,
  EntityKindName, EntityListing, EntityRef, ExportRequest, FilterIssue, FilterSpec, JobEvent, JobStarted, MessageDetail, PeekPageView, PeekRequest, PeekView,
  PickedPath, Profile, ProfileDraft, QueueInfo, RuleInfo, SessionListing, SubscriptionInfo, SessionSummary, SignedOut, SignInView, StoredProfile, StoredSignIn, TopicInfo,
} from "../types";
import { setUpdater } from "../updater";
import * as fx from "./fixtures";
import { accepts, summarize, textOf, without, type MockMessage } from "./messages";
import { createManageMock } from "./manage";
import { createOperateMock, UNHANDLED } from "./operate";

export type Scenario =
  | "default" | "empty" | "forbidden" | "missing-secret" | "network" | "sign-in-expired" | "many" | "many-topics"
  // Entra namespace discovery: nothing found, the listing fails, more namespaces than fit without a filter.
  | "no-namespaces" | "discovery-error" | "many-namespaces"
  // Signed in, with a profile in the guest tenant saved before that tenant's own sign-in: connecting to it
  // needs "Sign in to Northwind Traders".
  | "tenant-sign-in"
  // customs-docs holds 12,000 active messages (scan budgets); exports keep running until cancelled.
  | "many-messages" | "export-slow"
  // An export fails after its second tick (the connection resets); selected messages `export-missing` lost two
  // of the ones asked for (received since the grid read them); `export-refused` is refused before it starts.
  | "export-fails" | "export-missing" | "export-refused"
  // The session list's sources (returns-intake): the service lists none (`sessions-empty`), its list is cut at 10,000
  // (`sessions-capped`), or the call fails and the tally was cut short (`sessions-cut`).
  | "sessions-empty" | "sessions-capped" | "sessions-cut"
  // Every peek page is refused: the identity lacks Listen.
  | "peek-error"
  // The entities file chosen for an import can't be read (a value the parser refuses).
  | "import-broken"
  // A peek's first page arrives; the next one never does (screenshots of a view still reading).
  | "peek-slow"
  // Every message read for the detail pane has gone since the grid read it (received, expired or moved).
  | "message-gone"
  // The queues' counts as the service can give them: counted by peeking (`~N`), counted by peeking up to the cap
  // (`~N+`), or not reported at all (`?`); and a stored connection string that is scoped to another entity.
  | "counts-approx" | "counts-capped" | "counts-unknown" | "copy-scoped"
  // Operations (M2b): confirmations off production (`operate-ok`), on production with typed names (`operate-prod`; a move
  // asks for its source, then its target), one failed message (`operate-partial`), the connection closing after the
  // first message (`operate-stopped`).
  | "operate-ok" | "operate-prod" | "operate-partial" | "operate-stopped";

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** The namespace host from an `Endpoint=sb://<host>/` segment; the secret is never echoed back. */
function hostFromConnectionString(cs: string | null | undefined): string {
  const m = /Endpoint=sb:\/\/([^/;\s]+)/i.exec(cs ?? "");
  return m?.[1]?.toLowerCase() ?? "new.servicebus.windows.net";
}

/** `1 queue`, `4 queues`: the Rust activity lines use real plurals. */
const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** `1,284`: the Rust activity lines group digits. */
/** The views as the UI names them, which Rust uses in a job's title. */
const VIEW_NAME: Record<PeekView, string> = {
  active: "active messages",
  "dead-letter": "dead-letter queue",
  "transfer-dead-letter": "transfer dead-letter queue",
  scheduled: "scheduled messages",
  deferred: "deferred messages",
};
const thousands = (n: number) => n.toLocaleString("en-US");

/** As core `EntityPath::parse`: a queue, or `topic/subscriptions/name`; refused (`invalid`) when blank. */
function entityPath(input: string): string {
  const path = input.trim().replace(/^\/+|\/+$/g, "");
  if (!path || path.split("/").some((seg) => seg === "")) throw new EngineError("invalid", `\`${input}\` is not an entity; use \`queue\` or \`topic/subscriptions/subscription\``);
  return path;
}

/** One `name op literal` comparison of the property language (the mock understands these, joined by AND). */
const SIMPLE = /^\s*(sys\.)?([A-Za-z_][\w.-]*)\s*(=|<>|>=|<=|>|<)\s*('([^']*)'|-?\d+(?:\.\d+)?)\s*$/;
const clauses = (p: string) => p.split(/\s+AND\s+/i);

/** As core `FilterSpec::compile`: at most one problem per field, in the engine's words. */
export function issuesOf(f: FilterSpec): FilterIssue[] {
  const out: FilterIssue[] = [];
  const p = f.property?.trim();
  if (p && !clauses(p).every((c) => SIMPLE.test(c))) {
    const at = p.indexOf("==");
    out.push({ field: "property", message: at >= 0 ? `filter: unexpected \`=\` at column ${at + 2}` : `filter: expected a value at column ${p.length + 1}` });
  }
  if (f.body_regex?.trim()) {
    try {
      new RegExp(f.body_regex);
    } catch (e) {
      out.push({ field: "body_regex", message: `body regex: ${(e as Error).message}` });
    }
  }
  const path = f.body_json_path?.trim();
  if (path && !path.startsWith("$")) out.push({ field: "body_json_path", message: `body JSONPath \`${path}\`: expected \`$\` at position 1` });
  else if (!path && f.body_json_equals?.trim()) out.push({ field: "body_json_equals", message: "add a JSONPath to compare this value with" });
  return out;
}

function field(m: MockMessage, name: string, sys: boolean): unknown {
  if (sys) {
    const map: Record<string, unknown> = {
      subject: m.subject, label: m.subject, messageid: m.message_id, sessionid: m.session_id, deliverycount: m.delivery_count,
      sequencenumber: m.sequence_number, deadletterreason: m.dead_letter_reason, correlationid: m.correlation_id, contenttype: m.content_type,
    };
    return map[name.toLowerCase()];
  }
  const v = m.application_properties[name];
  return v && "value" in v ? v.value : undefined;
}

function compare(m: MockMessage, clause: string): boolean {
  const s = SIMPLE.exec(clause);
  if (!s) return false;
  const v = field(m, s[2], !!s[1]);
  if (v === undefined || v === null) return false;
  const lit: unknown = s[5] ?? Number(s[4]);
  const cmp = typeof v === "number" && typeof lit === "number" ? v - lit : String(v).localeCompare(String(lit));
  return { "=": cmp === 0, "<>": cmp !== 0, ">": cmp > 0, "<": cmp < 0, ">=": cmp >= 0, "<=": cmp <= 0 }[s[3] as "="];
}

/** The filter as the engine runs it: every set field must hold. */
function matches(m: MockMessage, f: FilterSpec): boolean {
  if (f.session_id && m.session_id !== f.session_id) return false;
  const p = f.property?.trim();
  if (p && !clauses(p).every((c) => compare(m, c))) return false;
  if (f.body_contains && !textOf(m).toLowerCase().includes(f.body_contains.toLowerCase())) return false;
  if (f.body_regex?.trim() && !new RegExp(f.body_regex).test(textOf(m))) return false;
  const path = f.body_json_path?.trim();
  if (path) {
    try {
      let node: unknown = JSON.parse(textOf(m));
      for (const part of path.replace(/^\$\.?/, "").split(".").filter(Boolean)) node = (node as Record<string, unknown> | null)?.[part];
      if (node === undefined) return false;
      // As core `BodyFilter::json`: the text as typed (blank means no expected value), as JSON or else a bare string.
      const typed = f.body_json_equals;
      if (typed?.trim()) {
        let want: unknown = typed;
        try {
          want = JSON.parse(typed);
        } catch {
          // a bare string
        }
        if (JSON.stringify(node) !== JSON.stringify(want)) return false;
      }
    } catch {
      return false;
    }
  }
  return true;
}

/** The sub-queue a view reads. */
const subOf = (view: PeekView): MockMessage["sub"] => (view === "dead-letter" ? "dlq" : view === "transfer-dead-letter" ? "tdlq" : "main");

/** Entities a profile that may not list the namespace still knows (its pins and `EntityPath`). */
const FORBIDDEN_KNOWN: EntityRef[] = [{ entity: "orders", subscription: null }, { entity: "shipment-events", subscription: "tracking-sync" }];

/** A remembered sign-in: an account's, or the same person's own sign-in in a guest tenant. */
interface MockSignIn { id: string; user_principal: string; tenant_id: string }

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
/** As Rust `same_identity`: the id, or a real user principal (never empty or the kit's "unknown"). */
const knownUpn = (upn: string) => upn.trim() !== "" && !same(upn.trim(), "unknown");
const sameIdentity = (a: MockSignIn, b: MockSignIn) => a.id === b.id || (knownUpn(a.user_principal) && same(a.user_principal, b.user_principal));
const GUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** As core's `normalize_tenant`: trimmed, a GUID lowercase, null when blank or the sign-in's own tenant. */
export function normalizeTenant(tenant: string | null | undefined, home: string | null): string | null {
  const t = tenant?.trim();
  if (!t || (home && same(t, home))) return null;
  return GUID.test(t) ? t.toLowerCase() : t;
}

export function createMockTransport(scenario: Scenario, delayMs = 0, seed: Record<string, unknown> = {}) {
  const listeners = new Map<string, Set<(p: unknown) => void>>();
  const calls: { cmd: string; args?: unknown }[] = [];
  const settings = new Map<string, unknown>(Object.entries(seed));
  let profiles: StoredProfile[] =
    scenario === "empty" || scenario === "no-namespaces" || scenario === "discovery-error" || scenario === "many-namespaces"
      ? []
      : structuredClone(scenario === "tenant-sign-in" ? [...fx.profiles, fx.guestProfile] : fx.profiles);
  let active: StoredProfile | null = null;
  let account: AccountView | null = scenario === "tenant-sign-in" ? { ...fx.accounts["oid-1"] } : null;
  /** Every remembered sign-in, and the ids whose refresh token exists. The default account signed in once
   *  before, so profiles bound to it work. A guest tenant's own sign-in (the same person, another id) is
   *  saved beside the account's and never becomes the current account, as in Rust. */
  const records = new Map<string, MockSignIn>(Object.values(fx.accounts).map((a) => [a.id, { ...a }]));
  const live = new Set<string>(["oid-1"]);
  let cancelSignIn: (() => void) | null = null;

  const current = (): MockSignIn | null => (account ? records.get(account.id) ?? null : null);
  /** As `AppState::live_sign_in_like`: a live sign-in made in `tenant` by `who`'s identity, `who` first. */
  const liveLike = (tenant: string, who: MockSignIn) =>
    [...records.values()]
      .filter((s) => same(s.tenant_id, tenant) && sameIdentity(s, who) && live.has(s.id))
      .sort((a, b) => Number(b.id === who.id) - Number(a.id === who.id))[0] ?? null;
  /** The current account's own sign-in in `tenant` (never another account's). */
  const tenantSignIn = (tenant: string) => {
    const c = current();
    return c ? liveLike(tenant, c) : null;
  };
  /** As `AppState::usable_sign_in`: the same person's own sign-in in the profile's tenant first, then the bound one. */
  const usable = (signInId: string, tenant: string | null) => {
    const bound = records.get(signInId) ?? null;
    if (!bound) return { bound, usable: null };
    const own = tenant && !same(tenant, bound.tenant_id) ? liveLike(tenant, bound) : null;
    if (own) return { bound, usable: own };
    if (live.has(bound.id)) return { bound, usable: bound };
    return { bound, usable: liveLike(bound.tenant_id, bound) };
  };
  const boundSignIn = (p: StoredProfile): BoundSignIn | null => {
    if (p.auth.kind !== "entra") return null;
    const u = usable(p.auth.sign_in_id, p.auth.tenant);
    const s = u.usable ?? u.bound;
    if (!s) return null;
    const c = current();
    return {
      id: s.id, user_principal: s.user_principal, tenant_id: s.tenant_id, live: u.usable !== null,
      same_account: c !== null && sameIdentity(c, s), own_tenant: c !== null && !same(c.tenant_id, s.tenant_id),
    };
  };
  const view = (p: StoredProfile): Profile => ({ ...structuredClone(p), bound_sign_in: boundSignIn(p) });
  /** As `entra_sign_in`: the current sign-in for its home tenant, else its own sign-in in the tenant, else the current one. */
  const resolveSignIn = (tenant: string | null | undefined) => {
    const c = current();
    const t = tenant?.trim();
    if (!c || !t || same(t, c.tenant_id)) return c;
    return tenantSignIn(t) ?? c;
  };

  const emit = (event: string, payload: unknown) => listeners.get(event)?.forEach((cb) => cb(payload));
  const activity = (action: string, started: number, error: EngineError | null = null) =>
    emit("activity", {
      at: new Date().toISOString(),
      action,
      duration_ms: Math.max(1, Math.round(performance.now() - started)),
      error: error ? { kind: error.kind, message: error.message } : null,
    } satisfies ActivityEntry);
  /** As `events::tracked_failures`: only a failure is logged (paged, polled and auto-refreshed reads). */
  async function quietly<T>(failed: string, started: number, run: () => T | Promise<T>): Promise<T> {
    try {
      return await run();
    } catch (e) {
      if (e instanceof EngineError && e.kind !== "cancelled") activity(failed, started, e);
      throw e;
    }
  }
  /** As `events::tracked`: success as `describe(value)`; a cancel as its message, with no error; a failure as `failed`. */
  async function tracked<T>(failed: string, started: number, run: () => T | Promise<T>, describe: (v: T) => string): Promise<T> {
    try {
      const v = await run();
      activity(describe(v), started);
      return v;
    } catch (e) {
      if (e instanceof EngineError) activity(e.kind === "cancelled" ? e.message.replace(/\.$/, "") : failed, started, e.kind === "cancelled" ? null : e);
      throw e;
    }
  }

  /** As `AppState::active_for`: the request's profile must be the connected one. */
  const connectedTo = (profileId: unknown) => {
    if (!active || active.id !== profileId) throw new EngineError("not_connected", "Choose a connection first.");
    return active;
  };
  const baseQueues = (): QueueInfo[] =>
    scenario === "many"
      ? fx.manyQueues
      : scenario === "many-messages"
        ? fx.queues.map((q) => (q.name === "customs-docs" ? { ...q, counts: fx.counts(12_000, 0) } : q))
        : scenario === "counts-approx" || scenario === "counts-capped"
          ? fx.queues.map((q) => ({ ...q, counts: q.counts && { ...q.counts, approximate: true, truncated: scenario === "counts-capped" } }))
          : scenario === "counts-unknown"
            ? fx.queues.map((q) => ({ ...q, counts: null }))
            : fx.queues;
  const baseTopics = (): TopicInfo[] => (scenario === "many-topics" ? fx.manyTopics : fx.topics);

  /** What manage operations changed in the namespace: entities created and deleted (by id), and rules. */
  const ns = { queues: [] as QueueInfo[], topics: [] as TopicInfo[], subs: {} as Record<string, SubscriptionInfo[]>, gone: new Set<string>(), rules: new Map<string, RuleInfo[]>() };
  const topicOfId = (id: string) => (id.includes("/subscriptions/") ? id.split("/subscriptions/")[0] : null);
  const isGone = (id: string) => ns.gone.has(id) || (topicOfId(id) !== null && ns.gone.has(topicOfId(id)!));
  const listedQueues = (): QueueInfo[] => [...baseQueues(), ...ns.queues].filter((q) => !isGone(q.name));
  const listedTopics = (): TopicInfo[] =>
    [...baseTopics(), ...ns.topics].filter((t) => !isGone(t.name)).map((t) => ({ ...t, subscription_count: subsOf(t.name).length }));
  const subsOf = (topic: string): SubscriptionInfo[] =>
    [...(fx.subscriptions[topic] ?? []), ...(ns.subs[topic] ?? [])].filter((x) => !isGone(`${topic}/subscriptions/${x.name}`));

  /** Messages operations took away (`entity|sequence`); the fixtures themselves are shared and never change. */
  const removed = new Set<string>();
  const present = (entity: string) => fx.messagesOf(entity, scenario).filter((m) => !removed.has(`${entity}|${m.sequence_number}`));
  const operateKind = (entity: string): EntityKindName | null =>
    listedQueues().some((q) => q.name === entity)
      ? "queue"
      : listedTopics().some((t) => t.name === entity)
        ? "topic"
        : (() => {
            const [topic, sub] = entity.split("/subscriptions/");
            return sub !== undefined && subsOf(topic).some((x) => x.name === sub) ? "subscription" : null;
          })();
  const operate = createOperateMock({
    scenario,
    tick: delayMs ? 90 : 0,
    emit: (event, payload) => emit(event, payload),
    connectedTo: (profileId) => connectedTo(profileId),
    messagesOf: present,
    remove: (entity, sequences) => sequences.forEach((n) => removed.add(`${entity}|${n}`)),
    matches: (m, f) => matches(m, f),
    kindOf: (entity) => operateKind(entity),
    countsOf: (entity) => countsOfEntity(entity),
  });
  function countsOfEntity(entity: string) {
      const [topic, sub] = entity.split("/subscriptions/");
      const c = sub === undefined ? listedQueues().find((q) => q.name === entity)?.counts : subsOf(topic).find((x) => x.name === sub)?.counts;
      if (!c) return null;
      // What is still there after operations took messages away.
      const gone = (k: MockMessage["sub"]) => fx.messagesOf(entity, scenario).filter((m) => m.sub === k && removed.has(`${entity}|${m.sequence_number}`)).length;
      return { active: Math.max(0, c.active - gone("main")), dead_letter: Math.max(0, c.dead_letter - gone("dlq")), transfer_dead_letter: Math.max(0, c.transfer_dead_letter - gone("tdlq")) };
  }

  const manage = createManageMock(
    {
      connectedTo: (profileId) => connectedTo(profileId),
      kindOf: (entity) => (isGone(entity) ? null : operateKind(entity)),
      add: (kind, entity) => {
        ns.gone.delete(entity);
        if (kind === "queue") ns.queues.push({ name: entity, status: "Active", counts: fx.counts(0, 0), size_in_bytes: 0, requires_session: false, max_delivery_count: 10, lock_duration: "PT1M", forward_to: null });
        else if (kind === "topic") ns.topics.push({ name: entity, status: "Active", size_in_bytes: 0, subscription_count: 0 });
        else {
          const [topic, name] = entity.split("/subscriptions/");
          (ns.subs[topic] ??= []).push({ topic, name, status: "Active", counts: fx.counts(0, 0), requires_session: false, max_delivery_count: 10, forward_to: null });
        }
      },
      remove: (entity) => void ns.gone.add(entity),
      count: (entity) => countsOfEntity(entity)?.active ?? null,
      rules: (entity) =>
        structuredClone(ns.rules.get(entity) ?? fx.rules[entity] ?? [{ name: "$Default", filter: { kind: "true" as const }, action: null, created_at: null }]),
      setRules: (entity, rules) => void ns.rules.set(entity, rules),
      openedFile: (pick) => {
        const names = operate.opened(pick);
        return names ? (names[0].startsWith("broken") ? "broken" : "<Entities />") : null;
      },
    },
    operate,
    () => (delayMs ? 600 : 25),
  );

  /** Export jobs: running ones (with their cancel), the files finished ones wrote, and the save dialog's picks. */
  interface Job { timer?: ReturnType<typeof setTimeout>; cancelling: boolean; finish: (error: ActivityError | null) => void }
  const running = new Map<string, Job>();
  const outputs = new Map<string, string>();
  const picks = new Map<string, string>();
  const job = (e: JobEvent) => emit("job", e);
  /** As `Jobs::cancel`: true while the job runs; it finishes (cancelled, no file) once it notices. `byClose`: as
   *  `Jobs::close_running`, it finishes as `stopped` (the connection closed) instead. */
  const cancelJob = (id: string, byClose = false) => {
    const j = running.get(id);
    if (!j) return false;
    if (!j.cancelling) {
      j.cancelling = true;
      clearTimeout(j.timer);
      const why = byClose ? { kind: "stopped", message: "Export stopped: the connection closed." } : { kind: "cancelled", message: "Export cancelled." };
      setTimeout(() => j.finish(why), 0);
    }
    return true;
  };
  /** Closing the connection (disconnect, another connect, signing out its sign-in) stops its exports. */
  const cancelRunning = () => {
    [...running.keys()].forEach((id) => cancelJob(id, true));
    operate.closeAll();
  };
  /** The prompt of the sign-in that is waiting, for "Open sign-in page". */
  let lastPrompt: { url: string } | null = null;

  async function handle(cmd: string, args: Record<string, unknown> = {}): Promise<unknown> {
    const started = performance.now();
    if (delayMs) await wait(delayMs);
    switch (cmd) {
      case "app_info":
        return { version: "0.0.1-mock", startup_error: null, updates: true };
      case "settings_get":
        return settings.get(args.key as string) ?? null;
      case "settings_set":
        if (args.value == null) settings.delete(args.key as string);
        else settings.set(args.key as string, structuredClone(args.value));
        return null;
      case "profiles_list":
        return profiles.map(view);
      case "profile_save": {
        const d = args.draft as ProfileDraft;
        const existing = d.id ? profiles.find((p) => p.id === d.id) : undefined;
        if (d.auth.kind === "connection-string" && !existing && !d.auth.connection_string?.trim())
          throw new EngineError("invalid", "paste the namespace's connection string");
        if (d.auth.kind === "entra" && !existing && !account)
          throw new EngineError("not_signed_in", "sign in with Entra ID first, then add the profile");
        // An Entra profile binds as in Rust; an edit rebinds only to the same person, or from a sign-in that no longer works.
        let auth: StoredProfile["auth"] = { kind: "connection-string" };
        if (d.auth.kind === "entra") {
          const resolved = resolveSignIn(d.auth.tenant);
          let bound = resolved;
          if (existing?.auth.kind === "entra") {
            const u = usable(existing.auth.sign_in_id, existing.auth.tenant);
            const now = u.usable ?? u.bound;
            // Another identity's binding stays, live or expired; only a gone record lets the current account in.
            if (!resolved || (now && !sameIdentity(now, resolved))) bound = u.bound;
          }
          const id = bound?.id ?? (existing?.auth.kind === "entra" ? existing.auth.sign_in_id : "oid-1");
          auth = { kind: "entra", sign_in_id: id, tenant: normalizeTenant(d.auth.tenant, bound?.tenant_id ?? null) };
        }
        const p: StoredProfile = {
          id: existing?.id ?? `p-${Math.random().toString(36).slice(2, 8)}`,
          name: d.name.trim(),
          host: d.auth.kind === "entra" ? (d.auth.namespace.includes(".") ? d.auth.namespace : `${d.auth.namespace}.servicebus.windows.net`) : existing?.host ?? hostFromConnectionString(d.auth.connection_string),
          auth,
          emulator: false,
          production: d.production,
          color: d.color ?? null,
          transport: d.transport,
          proxy: d.proxy ?? null,
          pinned_entities: d.pinned_entities,
        };
        if (profiles.some((o) => o.id !== p.id && o.name.toLowerCase() === p.name.toLowerCase())) {
          const e = new EngineError("invalid", `a profile named \`${p.name}\` already exists`);
          activity("Saving the connection failed", started, e);
          throw e;
        }
        profiles = existing ? profiles.map((o) => (o.id === p.id ? p : o)) : [...profiles, p];
        activity(`Saved connection ${p.name}`, started);
        return view(p);
      }
      case "profile_delete": {
        const p = profiles.find((o) => o.id === args.id);
        if (!p) throw new EngineError("not_found", "That profile no longer exists.");
        profiles = profiles.filter((o) => o.id !== args.id);
        if (active?.id === p.id) active = null;
        settings.delete(`shell:${p.id}`);
        activity(`Removed connection ${p.name}`, started);
        return null;
      }
      case "profile_test": {
        const p = profiles.find((o) => o.id === args.id);
        if (!p) throw new EngineError("not_found", "That profile no longer exists.");
        activity(`Tested ${p.name}: management ok, messaging ok`, started);
        return {
          host: p.host, emulator: false, transport: p.transport, auth: "SAS key reader",
          management: { status: "ok", detail: "listed 4 queues (Manage)" },
          messaging: { status: "ok", detail: "peeked customs-docs (Listen)" },
        };
      }
      case "account_current":
        return account;
      case "sign_in": {
        const prompt = args.deviceCode
          ? { kind: "device-code", url: "https://microsoft.com/devicelogin", user_code: "QX4T-9KLM", message: "Open the page and enter the code." }
          : { kind: "browser", url: "https://login.microsoftonline.com/", user_code: null, message: "Finish signing in in your browser." };
        emit("sign-in-prompt", prompt);
        // As Rust: kept while this sign-in waits, cleared when it returns unless a newer sign-in's prompt replaced it.
        const mine = { url: prompt.url };
        lastPrompt = mine;
        const cancelled = await new Promise<boolean>((resolve) => {
          const timer = setTimeout(() => {
            cancelSignIn = null;
            resolve(false);
          }, 400);
          cancelSignIn = () => {
            clearTimeout(timer);
            cancelSignIn = null;
            resolve(true);
          };
        });
        if (lastPrompt === mine) lastPrompt = null;
        if (cancelled) {
          // As in Rust: a cancelled operation is logged as its message, with no error.
          activity("Sign-in cancelled", started);
          throw new EngineError("cancelled", "Sign-in cancelled.");
        }
        const tenant = (args.tenant as string | null) || null;
        // Another tenant: that tenant's own sign-in by the same person (its MFA is now satisfied); the account stays.
        if (account && tenant && !same(tenant, account.tenant_id)) {
          const s: MockSignIn = { id: `oid-${tenant.replace(/^tenant-/, "")}`, user_principal: account.user_principal, tenant_id: tenant };
          records.set(s.id, s);
          live.add(s.id);
          activity(`Signed in to tenant ${tenant} as ${s.user_principal}`, started);
          return { account, tenant_sign_in: { tenant_id: tenant, user_principal: s.user_principal, same_account: true } } satisfies SignInView;
        }
        // The current account's home tenant: the same account signs in again and stays current.
        if (account && tenant) {
          live.add(account.id);
          activity(`Signed in as ${account.user_principal}`, started);
          return { account, tenant_sign_in: null } satisfies SignInView;
        }
        // Signing in again while signed in picks the other fixture account.
        account = { ...(account?.id === "oid-1" ? fx.accounts["oid-2"] : fx.accounts["oid-1"]) };
        live.add(account.id);
        activity(`Signed in as ${account.user_principal}`, started);
        return { account, tenant_sign_in: null } satisfies SignInView;
      }
      case "list_tenants": {
        if (!account) throw new EngineError("not_signed_in", "Sign in with Entra ID to list your namespaces.");
        const tenants = fx.tenantsFor(account.tenant_id).map((t) => (t.home ? t : { ...t, sign_in_upn: tenantSignIn(t.tenant_id)?.user_principal ?? null }));
        activity(`Listed ${count(tenants.length, "tenant", "tenants")}`, started);
        return tenants;
      }
      case "discover_namespaces": {
        if (!account) throw new EngineError("not_signed_in", "Sign in with Entra ID to list your namespaces.");
        const asked = (args.tenant as string | null) || null;
        const home = asked === null || asked === account.tenant_id;
        const label = home ? "the home tenant" : `tenant ${asked}`;
        const fail = (e: EngineError, action = `Finding namespaces in ${label} failed`) => {
          activity(action, started, e);
          return e;
        };
        if (!home && asked === fx.GUEST_TENANT && !tenantSignIn(asked))
          throw fail(new EngineError("mfa_required", "This tenant requires multi-factor sign-in. Sign in to it to list its namespaces."));
        if (scenario === "discovery-error") throw fail(new EngineError("network", "network error: connection reset by peer"));
        const found = !home
          ? asked === fx.GUEST_TENANT ? fx.guestDiscovery : { namespaces: [], skipped: [] }
          : scenario === "no-namespaces"
            ? { namespaces: [], skipped: [] }
            : scenario === "many-namespaces"
              ? fx.manyDiscoveryFor(account.tenant_id)
              : fx.discoveryFor(account.tenant_id);
        activity(`Found ${count(found.namespaces.length, "namespace", "namespaces")} in ${label}`, started);
        return structuredClone(found);
      }
      case "sign_in_cancel":
        cancelSignIn?.();
        return null;
      case "connect": {
        const p = profiles.find((o) => o.id === args.profileId);
        if (!p) throw new EngineError("not_found", "That profile no longer exists.");
        if (scenario === "missing-secret") {
          const e = new EngineError("invalid", `profile \`${p.name}\` has no stored connection string; edit the connection and paste it again`);
          activity(`Connecting to ${p.name} failed`, started, e);
          throw e;
        }
        if (p.auth.kind === "entra") {
          const u = usable(p.auth.sign_in_id, p.auth.tenant).usable;
          // A sign-in that no longer works, or a sign-in from elsewhere asking for the guest tenant, which wants its own (MFA) sign-in.
          const asked = p.auth.tenant;
          const e = !u
            ? new EngineError("not_signed_in", `the sign-in used by profile \`${p.name}\` has expired; sign in again`)
            : asked && same(asked, fx.GUEST_TENANT) && !same(u.tenant_id, asked)
              ? new EngineError("not_signed_in", `${u.user_principal}: sign in again (interaction_required: AADSTS50076: you must use multi-factor authentication)`)
              : null;
          if (e) {
            activity(`Connecting to ${p.name} failed`, started, e);
            throw e;
          }
        }
        // Connecting closes the previous connection, which cancels its exports.
        if (active) cancelRunning();
        active = p;
        activity(`Connected to ${p.name}`, started);
        return {
          profile_id: p.id, profile_name: p.name, host: p.host, emulator: p.emulator,
          auth: p.auth.kind === "entra" ? "Entra ID ops@contoso.example" : "SAS key reader",
          transport: p.transport, production: p.production,
        } satisfies ConnectionView;
      }
      case "disconnect":
        if (active) cancelRunning();
        active = null;
        activity("Disconnected", started);
        return null;
      case "list_entities": {
        if (!active || active.id !== args.profileId) throw new EngineError("not_connected", "Choose a connection first.");
        if (scenario === "network") {
          const e = new EngineError("network", "network error: connection reset by peer");
          activity("Listing the namespace failed", started, e);
          throw e;
        }
        if (scenario === "sign-in-expired") {
          const e = new EngineError("not_signed_in", "not signed in: the refresh token has expired; sign in again");
          activity("Listing the namespace failed", started, e);
          throw e;
        }
        if (scenario === "forbidden") {
          activity(`Listing not allowed; showing ${count(2, "pinned entity", "pinned entities")}`, started);
          return {
            queues: [], topics: [],
            limited: "This identity can't list the namespace (needs Manage,EntityRead). Showing pinned entities.",
            known: FORBIDDEN_KNOWN,
          } satisfies EntityListing;
        }
        const queues = listedQueues();
        const topics = listedTopics();
        activity(`Listed ${count(queues.length, "queue", "queues")} and ${count(topics.length, "topic", "topics")}`, started);
        return { queues, topics, limited: null, known: [] } satisfies EntityListing;
      }
      case "list_subscriptions": {
        if (!active || active.id !== args.profileId) throw new EngineError("not_connected", "Choose a connection first.");
        // Read on every expand and polled by a topic's Overview: as Rust, only a failure is logged.
        return quietly(`Listing subscriptions of ${args.topic} failed`, started, () => subsOf(args.topic as string));
      }
      case "peek_page": {
        // As `browse::prepare`: the entity and the filter are checked before anything else, and not logged.
        const r = args.request as PeekRequest;
        const entity = entityPath(r.entity);
        const bad = issuesOf(r.filter ?? {});
        if (bad.length) throw new EngineError("invalid", bad[0].message);
        connectedTo(args.profileId);
        const tail = r.from.kind === "end";
        const want = Math.min(Math.max(r.max, 1), tail ? 5_000 : 500);
        const scanLimit = Math.min(Math.max(r.scan_limit ?? (tail ? 100_000 : 5_000), 1), tail ? 100_000 : 5_000);
        const filter = r.filter ?? {};
        const hit = (m: MockMessage) => accepts(r.view, m) && matches(m, filter);
        // A load is up to 20 pages: as Rust, a page is logged only when it fails.
        if (scenario === "peek-slow" && r.from.kind === "sequence") await new Promise(() => {});
        return quietly(`Peeking ${entity} failed`, started, () => {
          if (scenario === "peek-error")
            throw new EngineError("forbidden", "access denied (needs Listen): Unauthorized access. 'Listen' claim(s) are required to perform this operation.");
          const all = present(entity).filter((m) => m.sub === subOf(r.view));
          if (tail) {
            // The newest first, back towards the oldest; the budget may run out inside the newest window.
            const out: MockMessage[] = [];
            let scanned = 0;
            for (let j = all.length - 1; j >= 0 && out.length < want && scanned < scanLimit; j--) {
              scanned++;
              if (hit(all[j])) out.unshift(all[j]);
            }
            const exhausted = out.length >= want || scanned >= Math.min(Math.max(want, 64), all.length);
            return { messages: out.map(summarize), scanned, scan_limit: scanLimit, next_sequence: null, exhausted } satisfies PeekPageView;
          }
          const from = r.from.kind === "sequence" ? Math.max(0, r.from.sequence) : 0;
          const out: MockMessage[] = [];
          let scanned = 0;
          let i = all.findIndex((m) => m.sequence_number >= from);
          if (i < 0) i = all.length;
          for (; i < all.length && out.length < want && scanned < scanLimit; i++) {
            scanned++;
            if (hit(all[i])) out.push(all[i]);
          }
          const exhausted = i >= all.length;
          return { messages: out.map(summarize), scanned, scan_limit: scanLimit, next_sequence: exhausted ? null : all[i].sequence_number, exhausted } satisfies PeekPageView;
        });
      }
      case "message_detail": {
        const entity = entityPath(args.entity as string);
        connectedTo(args.profileId);
        return quietly(`Reading message ${args.sequence} of ${entity} failed`, started, () => {
          // As Rust: read through the row's session when given, so another session's message is not there.
          const session = (args.sessionId as string | null | undefined) ?? null;
          const m = present(entity).find(
            (x) => x.sequence_number === args.sequence && accepts(args.view as PeekView, x) && (session === null || x.session_id === session),
          );
          if (!m || scenario === "message-gone") return null;
          return structuredClone(without(m, "sub")) satisfies MessageDetail;
        });
      }
      case "check_filter":
        return issuesOf(args.filter as FilterSpec);
      case "entity_details": {
        connectedTo(args.profileId);
        const kind = args.kind as EntityKindName;
        const id = args.entity as string;
        // Overview polls this: as Rust, only failures are logged.
        return quietly(`Reading the details of ${id} failed`, started, () => {
          const [topic, subName] = kind === "subscription" ? id.split("/subscriptions/") : [null, null];
          const info =
            kind === "queue"
              ? listedQueues().find((x) => x.name === id)
              : kind === "topic"
                ? listedTopics().find((x) => x.name === id)
                : subsOf(topic ?? "").find((x) => x.name === subName);
          if (!info) throw new EngineError("not_found", `not found: ${kind} \`${id}\``);
          const c: EntityCounts | null = "counts" in info ? info.counts : null;
          return {
            kind,
            name: subName ?? id,
            topic,
            status: info.status,
            counts: c,
            size_in_bytes: "size_in_bytes" in info ? info.size_in_bytes : null,
            properties: fx.details(kind, c, "requires_session" in info && info.requires_session, ("forward_to" in info && info.forward_to) || ""),
          } satisfies EntityDetails;
        });
      }
      case "list_rules": {
        const id = entityPath(args.entity as string);
        const [topic, sub] = id.split("/subscriptions/");
        if (sub === undefined) throw new EngineError("invalid", `${args.entity} is not a subscription; only subscriptions have rules.`);
        connectedTo(args.profileId);
        return tracked(
          `Listing the rules of ${id} failed`,
          started,
          () => {
            if (ns.rules.has(id)) return structuredClone(ns.rules.get(id)!);
            if (!subsOf(topic).some((x) => x.name === sub) && !FORBIDDEN_KNOWN.some((k) => k.entity === topic && k.subscription === sub))
              throw new EngineError("not_found", `not found: subscription \`${id}\``);
            const defaultOnly: RuleInfo[] = [{ name: "$Default", filter: { kind: "sql", expression: "1=1" }, action: null, created_at: null }];
            return structuredClone(fx.rules[id] ?? defaultOnly);
          },
          (r) => `Listed ${count(r.length, "rule", "rules")} of ${id}`,
        );
      }
      case "list_sessions": {
        const id = entityPath(args.entity as string);
        connectedTo(args.profileId);
        return tracked(
          `Listing the sessions of ${id} failed`,
          started,
          () => {
            const main = fx.messagesOf(id, scenario).filter((m) => m.sub === "main");
            // The tally: messages per session in the main queue.
            const per = new Map<string, MockMessage[]>();
            for (const m of main) if (m.session_id) per.set(m.session_id, [...(per.get(m.session_id) ?? []), m]);
            // The broker lists returns-intake's sessions, and one that has state but no messages, unless the scenario
            // takes the service's list away (`sessions-empty`, `sessions-cut`).
            const fromBroker = scenario !== "sessions-empty" && scenario !== "sessions-cut";
            const broker = id === "returns-intake" && fromBroker ? [...fx.RMA, "RMA-0999"] : [];
            const ids = [...new Set([...per.keys(), ...broker])].sort((a, b) => a.localeCompare(b));
            const sessions: SessionSummary[] = ids.map((sid) => {
              const ms = per.get(sid) ?? [];
              return { id: sid, messages: ms.length, first_enqueued: ms[0]?.enqueued_time ?? null, last_enqueued: ms.at(-1)?.enqueued_time ?? null, in_broker: broker.includes(sid) };
            });
            const cut = scenario === "sessions-cut";
            return {
              sessions,
              broker: cut
                ? { state: "unavailable", kind: "forbidden" }
                : broker.length
                  ? { state: "listed", capped: scenario === "sessions-capped" }
                  : { state: "empty" },
              scanned: cut ? 10_000 : main.length,
              truncated: cut,
            } satisfies SessionListing;
          },
          (l) => `Listed ${count(l.sessions.length, "session", "sessions")} of ${id}`,
        );
      }
      case "dashboard": {
        const p = connectedTo(args.profileId);
        // Auto-refreshed: as Rust, only failures are logged (the first load included).
        return quietly("Loading the dashboard failed", started, () => {
          if (scenario === "network") throw new EngineError("network", "network error: connection reset by peer");
          if (scenario === "sign-in-expired") throw new EngineError("not_signed_in", "not signed in: the refresh token has expired; sign in again");
          if (scenario === "forbidden") {
            const known = [...FORBIDDEN_KNOWN, ...p.pinned_entities.map((e) => ({ entity: e, subscription: null }))];
            const rows: DashboardRow[] = known
              .filter((k, i) => known.findIndex((o) => o.entity === k.entity && o.subscription === k.subscription) === i)
              .map((k) =>
                k.subscription
                  ? { kind: "subscription", id: `${k.entity}/subscriptions/${k.subscription}`, name: k.subscription, topic: k.entity, status: null, counts: null }
                  : { kind: "queue", id: k.entity, name: k.entity, topic: null, status: null, counts: null },
              );
            return {
              rows,
              failures: [],
              limited: "This identity can't list the namespace (needs Manage,EntityRead). Showing pinned entities without counts.",
            } satisfies DashboardView;
          }
          const rows: DashboardRow[] = [
            ...listedQueues().map((q) => ({ kind: "queue" as const, id: q.name, name: q.name, topic: null, status: q.status, counts: q.counts })),
            ...listedTopics().flatMap((t) =>
              subsOf(t.name).map((s) => ({
                kind: "subscription" as const, id: `${s.topic}/subscriptions/${s.name}`, name: s.name, topic: s.topic, status: s.status, counts: s.counts,
              })),
            ),
          ];
          return { rows, failures: [], limited: null } satisfies DashboardView;
        });
      }
      case "pick_export_path": {
        // The native save dialog: the path stays here under a one-time pick id.
        const name = suggestedFileName(args.entity as string, args.view as PeekView, args.format as string);
        const dir = "/Users/ops/Downloads";
        const pick = `pick-${Math.random().toString(36).slice(2, 12)}`;
        // As `Jobs::pick`: a new pick replaces any unspent one (one export sheet is open at a time).
        picks.clear();
        picks.set(pick, `${dir}/${name}`);
        return { pick, name, dir } satisfies PickedPath;
      }
      case "export_start": {
        const r = args.request as ExportRequest;
        // `deny_unknown_fields`: Tauri refuses the arguments before the command runs.
        const unknown = Object.keys(r).find((k) => !["entity", "view", "scope", "format", "pick", "total_hint"].includes(k));
        if (unknown) throw new EngineError("unknown", `invalid args \`request\` for command \`export_start\`: unknown field \`${unknown}\``);
        connectedTo(args.profileId);
        const path = picks.get(r.pick);
        if (!path) throw new EngineError("invalid", "Choose where to save the export.");
        // As `export::plan`; a request refused here keeps its pick.
        const entity = entityPath(r.entity);
        if (!["json", "ndjson", "jsonl"].includes(String(r.format).trim().toLowerCase()))
          throw new EngineError("invalid", `unknown export format \`${r.format}\`; use json or ndjson`);
        // `export-refused`: the engine refuses a request the sheet let through (the filter changed behind it).
        if (scenario === "export-refused") throw new EngineError("invalid", "body regex: Invalid regular expression: /(/: Unterminated group");
        if (r.scope.kind === "all") {
          const bad = issuesOf(r.scope.filter ?? {});
          if (bad.length) throw new EngineError("invalid", bad[0].message);
        } else if (r.scope.sequences.length === 0) throw new EngineError("invalid", "Select the messages to export.");
        picks.delete(r.pick);
        const name = path.split("/").at(-1)!;
        const title = `Export ${entity} (${VIEW_NAME[r.view]})`;
        const inView = fx.messagesOf(entity, scenario).filter((m) => accepts(r.view, m));
        let found: MockMessage[];
        let total: number | null;
        let note = "";
        let missingCount = 0;
        const filtered = r.scope.kind === "all" && Object.values(r.scope.filter ?? {}).some((v) => typeof v === "string" && v.trim() !== "");
        if (r.scope.kind === "selected") {
          // `export-missing`: the last two selected messages were received since the grid read them.
          const wanted = [...new Set(r.scope.sequences)];
          const lost = scenario === "export-missing" ? new Set(wanted.slice(-2)) : new Set<number>();
          // As Rust: through the rows' session when given, keeping only that session's messages.
          const session = r.scope.session_id ?? null;
          found = inView.filter((m) => wanted.includes(m.sequence_number) && !lost.has(m.sequence_number) && (session === null || m.session_id === session));
          total = wanted.length;
          missingCount = wanted.length - found.length;
          if (missingCount === 1) note = "; 1 selected message was no longer there";
          else if (missingCount > 1) note = `; ${missingCount} selected messages were no longer there`;
        } else {
          const { filter = {}, from_sequence = 0, max = null } = r.scope;
          found = inView.filter((m) => m.sequence_number >= Math.max(0, from_sequence) && matches(m, filter));
          if (max != null) found = found.slice(0, max);
          const hint = r.total_hint ?? null;
          total = max != null && hint != null ? Math.min(max, hint) : (max ?? hint);
        }
        const id = `job-${Math.random().toString(36).slice(2, 10)}`;
        job({ type: "started", id, kind: "export", title, total, file_name: name });
        let written = 0;
        const entry: Job = {
          cancelling: false,
          finish: (error) => {
            clearTimeout(entry.timer);
            running.delete(id);
            if (error) {
              const e = new EngineError(error.kind, error.message);
              // As `tracked`: a cancel (or a stop because the connection closed) is logged as its message without an error.
              const stop = error.kind === "cancelled" || error.kind === "stopped";
              activity(stop ? error.message.replace(/\.$/, "") : `${title} to ${name} failed`, started, stop ? null : e);
              job({ type: "finished", id, error, report: null, summary: null, output: null, written: null, missing: null, file_name: null });
              return;
            }
            const summary = `Exported ${thousands(written)} ${written === 1 ? "message" : "messages"} to ${name}${note}`;
            outputs.set(id, path);
            activity(summary, started);
            job({ type: "finished", id, error: null, report: null, summary, output: path, written, missing: missingCount, file_name: name });
          },
        };
        // A page at a time (selected messages in one write); `export-slow` writes one message a tick and never ends.
        const chunk =
          scenario === "export-slow" ? 1 : scenario === "export-fails" ? 5 : r.scope.kind === "selected" ? found.length : Math.max(1, Math.min(1000, Math.ceil(found.length / 3)));
        // A filtered export examines more messages than it writes.
        const ratio = filtered ? Math.ceil(inView.length / Math.max(1, found.length)) : 1;
        const tick = () => {
          if (entry.cancelling) return;
          // `export-fails`: the connection resets on the second page.
          if (scenario === "export-fails" && written >= chunk) return entry.finish({ kind: "network", message: "network error: connection reset by peer" });
          written = Math.min(found.length, written + chunk);
          job({ type: "progress", id, done: written, total, scanned: Math.min(inView.length, written * ratio) });
          if (scenario !== "export-slow" && written >= found.length) entry.finish(null);
          else entry.timer = setTimeout(tick, 150);
        };
        entry.timer = setTimeout(tick, 150);
        running.set(id, entry);
        return { id } satisfies JobStarted;
      }
      case "job_cancel":
        return cancelJob(args.id as string) || operate.cancel(args.id as string);
      case "job_reveal":
        if (!outputs.has(args.id as string)) throw new EngineError("not_found", "That export has no file to show.");
        return null;
      case "profile_test_draft": {
        const d = args.draft as ProfileDraft;
        const name = d.name.trim();
        const existing = d.id ? profiles.find((p) => p.id === d.id) : undefined;
        // As Rust: everything runs inside `tracked`, so a refused draft is logged as a failed test. The
        // connection string is used for this test only and never echoed or logged.
        return tracked(
          "Testing the connection failed",
          started,
          (): ConnectionReport => {
            let host: string;
            let auth = "SAS key reader";
            if (d.auth.kind === "connection-string") {
              if (d.auth.connection_string?.trim()) host = hostFromConnectionString(d.auth.connection_string);
              else if (existing?.auth.kind === "connection-string") host = existing.host;
              else throw new EngineError("invalid", "paste the namespace's connection string");
            } else {
              // As `draft_sign_in`: an edited profile tests with the sign-in connecting would use; a new one with
              // the sign-in saving would bind. A profile whose sign-in record is gone has none until it is saved.
              const s = existing?.auth.kind === "entra" ? usable(existing.auth.sign_in_id, d.auth.tenant ?? null).usable : resolveSignIn(d.auth.tenant);
              if (!s) throw new EngineError("not_signed_in", "not signed in: sign in with Entra ID first, then test the connection");
              host = d.auth.namespace.includes(".") ? d.auth.namespace : `${d.auth.namespace}.servicebus.windows.net`;
              auth = `Entra ID ${s.user_principal}`;
            }
            // As a namespace the identity can't manage: Management fails, and with no queue found Messaging is skipped.
            if (scenario === "forbidden")
              return {
                host, emulator: false, transport: d.transport, auth,
                management: { status: "failed", kind: "forbidden", message: "access denied (needs Manage): Unauthorized access. 'Manage' claim(s) are required to perform this operation." },
                messaging: { status: "skipped", reason: "no queue could be discovered because the management check failed; pass an entity to test Listen" },
              };
            return {
              host, emulator: false, transport: d.transport, auth,
              management: { status: "ok", detail: "listed 4 queues (Manage)" }, messaging: { status: "ok", detail: "peeked customs-docs (Listen)" },
            };
          },
          (r) => `Tested ${name} (unsaved): management ${r.management.status}, messaging ${r.messaging.status}`,
        );
      }
      case "copy_entity_connection_string": {
        const p = profiles.find((o) => o.id === args.profileId);
        if (!p) throw new EngineError("not_found", "That profile no longer exists.");
        const entity = entityPath(args.entity as string);
        // The string goes to the clipboard in Rust; nothing comes back.
        return tracked(
          `Copying the connection string for ${entity} failed`,
          started,
          () => {
            if (p.auth.kind === "entra") throw new EngineError("invalid", "Entra ID connections have no connection string. Copy the URL instead.");
            if (scenario === "copy-scoped") throw new EngineError("invalid", "this connection string only works for `orders`");
            if (scenario === "missing-secret") throw new EngineError("invalid", `${p.name} has no stored connection string. Edit the connection and paste it again.`);
            if (!/^[A-Za-z0-9._/~-]+$/.test(entity))
              throw new EngineError("invalid", `\`${entity}\` is not a valid entity name (letters, digits, \`.\`, \`_\`, \`-\`, \`~\` and \`/\` only)`);
            return null;
          },
          () => `Copied the connection string for ${entity}`,
        );
      }
      case "sign_ins_list": {
        // As Rust `sign_ins`, over M1b's mock model: every record, ordered by id; whose it is; and the profiles
        // that connect with it now (`boundSignIn`, which may be the same person's tenant sign-in).
        const c = current();
        const names = fx.accounts as Record<string, { display_name: string | null } | undefined>;
        return [...records.values()]
          .sort((a, b) => a.id.localeCompare(b.id))
          .map((s) => ({
            id: s.id,
            user_principal: s.user_principal,
            display_name: names[s.id]?.display_name ?? null,
            tenant_id: s.tenant_id,
            current: account?.id === s.id,
            signed_in: live.has(s.id),
            same_account: c !== null && sameIdentity(c, s),
            profiles: profiles.filter((p) => boundSignIn(p)?.id === s.id).map((p) => p.name),
          })) satisfies StoredSignIn[];
      }
      case "sign_out": {
        const s = records.get(args.id as string);
        if (!s) throw new EngineError("not_found", "That sign-in is already gone.");
        // Decided before the record goes: the active connection closes when its profile uses this sign-in.
        const uses = (p: StoredProfile) => p.auth.kind === "entra" && (p.auth.sign_in_id === s.id || usable(p.auth.sign_in_id, p.auth.tenant).usable?.id === s.id);
        const closing = active && uses(active) ? active : null;
        records.delete(s.id);
        live.delete(s.id);
        if (account?.id === s.id) account = null;
        if (closing) {
          cancelRunning();
          active = null;
        }
        activity(closing ? `Signed out ${s.user_principal} and closed ${closing.name}` : `Signed out ${s.user_principal}`, started);
        return { closed_profile_id: closing?.id ?? null } satisfies SignedOut;
      }
      case "open_sign_in_page":
        if (!lastPrompt) throw new EngineError("not_found", "No sign-in is waiting.");
        if (!lastPrompt.url.startsWith("https://")) throw new EngineError("invalid", "The sign-in page is not an https address, so it was not opened.");
        return null;
      default: {
        const r = operate.handle(cmd, args);
        if (r !== UNHANDLED) return r;
        const m = manage.handle(cmd, args);
        if (m === UNHANDLED) throw new EngineError("unknown", `mock: unknown command ${cmd}`);
        return m;
      }
    }
  }

  const transport: IpcTransport & { calls: typeof calls; external: typeof operate.external } = {
    calls,
    /** A change made by the CLI or an agent, written to the shared activity log. */
    external: operate.external,
    async invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
      calls.push(args === undefined ? { cmd } : { cmd, args });
      return (await handle(cmd, args)) as T;
    },
    async listen<T>(event: string, cb: (payload: T) => void): Promise<Unlisten> {
      const set = listeners.get(event) ?? new Set();
      set.add(cb as (p: unknown) => void);
      listeners.set(event, set);
      return () => set.delete(cb as (p: unknown) => void);
    },
  };
  return transport;
}

/** Mock builds: `?scenario=` picks the scenario, `?update=<mode>` offers a fake update, `?autoconnect=<profile id>`
 *  seeds the last-used profile so the app opens connected, and `?seed=<json object>` seeds further settings
 *  (screenshots use it to start with saved tabs). */
export function installMock(): void {
  const params = new URLSearchParams(window.location.search);
  const scenario = (params.get("scenario") ?? "default") as Scenario;
  const autoconnect = params.get("autoconnect");
  let seed: Record<string, unknown> = {};
  try {
    const raw = JSON.parse(params.get("seed") ?? "{}") as unknown;
    if (typeof raw === "object" && raw !== null && !Array.isArray(raw)) seed = raw as Record<string, unknown>;
  } catch {
    // an unreadable seed is ignored
  }
  // `?update=1` offers an update that installs. Screenshot modes: `downloading` stops the install at 60%,
  // `check-error` fails the check, `install-error` fails the install part-way (both with a plain string, as the plugin does).
  const update = params.get("update");
  if (update === "1" || update === "downloading" || update === "check-error" || update === "install-error") {
    setUpdater({
      check: async () => {
        if (update === "check-error") throw "Could not reach the update server.";
        return { version: "0.1.0", notes: "Faster tree loading.\nThe PROD badge now shows in every theme.", date: "2026-10-20" };
      },
      install: async (cb, onInstalling) => {
        for (const p of update === "1" ? [20, 60, 100] : [20, 60]) {
          await wait(150);
          cb(p);
        }
        if (update === "downloading") await new Promise(() => {});
        if (update === "install-error") throw "The download was interrupted.";
        onInstalling?.();
        await wait(150);
      },
    });
  }
  setTransport(createMockTransport(scenario, 120, autoconnect ? { ...seed, last_profile: autoconnect } : seed));
}
