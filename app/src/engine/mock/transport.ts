import { EngineError, setTransport, type IpcTransport, type Unlisten } from "../ipc";
import type { AccountView, ActivityEntry, BoundSignIn, ConnectionView, EntityListing, Profile, ProfileDraft, SignInView, StoredProfile } from "../types";
import { setUpdater } from "../updater";
import * as fx from "./fixtures";

export type Scenario =
  | "default" | "empty" | "forbidden" | "missing-secret" | "network" | "sign-in-expired" | "many" | "many-topics"
  // Entra namespace discovery: nothing found, the listing fails, more namespaces than fit without a filter.
  | "no-namespaces" | "discovery-error" | "many-namespaces"
  // Signed in, with a profile in the guest tenant saved before that tenant's own sign-in: connecting to it
  // needs "Sign in to Northwind Traders".
  | "tenant-sign-in";

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** The namespace host from an `Endpoint=sb://<host>/` segment; the secret is never echoed back. */
function hostFromConnectionString(cs: string | null | undefined): string {
  const m = /Endpoint=sb:\/\/([^/;\s]+)/i.exec(cs ?? "");
  return m?.[1]?.toLowerCase() ?? "new.servicebus.windows.net";
}

/** `1 queue`, `4 queues`: the Rust activity lines use real plurals. */
const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** A remembered sign-in: an account's, or the same person's own sign-in in a guest tenant. */
interface MockSignIn { id: string; user_principal: string; tenant_id: string }

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
const sameIdentity = (a: MockSignIn, b: MockSignIn) => a.id === b.id || same(a.user_principal, b.user_principal);
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

  async function handle(cmd: string, args: Record<string, unknown> = {}): Promise<unknown> {
    const started = performance.now();
    if (delayMs) await wait(delayMs);
    switch (cmd) {
      case "app_info":
        return { version: "0.0.1-mock", startup_error: null };
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
            if (!resolved || (now && u.usable && !sameIdentity(now, resolved))) bound = u.bound;
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
        emit("sign-in-prompt", args.deviceCode
          ? { kind: "device-code", url: "https://microsoft.com/devicelogin", user_code: "QX4T-9KLM", message: "Open the page and enter the code." }
          : { kind: "browser", url: "https://login.microsoftonline.com/", user_code: null, message: "Finish signing in in your browser." });
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
        active = p;
        activity(`Connected to ${p.name}`, started);
        return {
          profile_id: p.id, profile_name: p.name, host: p.host, emulator: p.emulator,
          auth: p.auth.kind === "entra" ? "Entra ID ops@contoso.example" : "SAS key reader",
          transport: p.transport, production: p.production,
        } satisfies ConnectionView;
      }
      case "disconnect":
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
            known: [{ entity: "orders", subscription: null }, { entity: "shipment-events", subscription: "tracking-sync" }],
          } satisfies EntityListing;
        }
        const queues = scenario === "many" ? fx.manyQueues : fx.queues;
        const topics = scenario === "many-topics" ? fx.manyTopics : fx.topics;
        activity(`Listed ${count(queues.length, "queue", "queues")} and ${count(topics.length, "topic", "topics")}`, started);
        return { queues, topics, limited: null, known: [] } satisfies EntityListing;
      }
      case "list_subscriptions": {
        if (!active || active.id !== args.profileId) throw new EngineError("not_connected", "Choose a connection first.");
        const subs = fx.subscriptions[args.topic as string] ?? [];
        activity(`Listed ${count(subs.length, "subscription", "subscriptions")} of ${args.topic}`, started);
        return subs;
      }
      default:
        throw new EngineError("unknown", `mock: unknown command ${cmd}`);
    }
  }

  const transport: IpcTransport & { calls: typeof calls } = {
    calls,
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
