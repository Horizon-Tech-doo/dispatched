import { EngineError, setTransport, type IpcTransport, type Unlisten } from "../ipc";
import type { AccountView, ActivityEntry, ConnectionView, EntityListing, Profile, ProfileDraft } from "../types";
import * as fx from "./fixtures";

export type Scenario = "default" | "empty" | "forbidden" | "missing-secret" | "network" | "many" | "many-topics";

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

export function createMockTransport(scenario: Scenario, delayMs = 0, seed: Record<string, unknown> = {}) {
  const listeners = new Map<string, Set<(p: unknown) => void>>();
  const calls: { cmd: string; args?: unknown }[] = [];
  const settings = new Map<string, unknown>(Object.entries(seed));
  let profiles: Profile[] = scenario === "empty" ? [] : structuredClone(fx.profiles);
  let active: Profile | null = null;
  let account: AccountView | null = null;

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
        return structuredClone(profiles);
      case "profile_save": {
        const d = args.draft as ProfileDraft;
        const existing = d.id ? profiles.find((p) => p.id === d.id) : undefined;
        const p: Profile = {
          id: existing?.id ?? `p-${Math.random().toString(36).slice(2, 8)}`,
          name: d.name.trim(),
          host: d.auth.kind === "entra" ? (d.auth.namespace.includes(".") ? d.auth.namespace : `${d.auth.namespace}.servicebus.windows.net`) : existing?.host ?? "new.servicebus.windows.net",
          auth: d.auth.kind === "entra" ? { kind: "entra", sign_in_id: account?.id ?? "oid-1", tenant: d.auth.tenant ?? null } : { kind: "connection-string" },
          emulator: false,
          production: d.production,
          color: d.color ?? null,
          transport: d.transport,
          proxy: d.proxy ?? null,
          pinned_entities: d.pinned_entities,
        };
        if (profiles.some((o) => o.id !== p.id && o.name.toLowerCase() === p.name.toLowerCase())) {
          const e = new EngineError("invalid", `a profile named \`${p.name}\` already exists`);
          activity("Saving the profile failed", started, e);
          throw e;
        }
        profiles = existing ? profiles.map((o) => (o.id === p.id ? p : o)) : [...profiles, p];
        activity(`Saved profile ${p.name}`, started);
        return structuredClone(p);
      }
      case "profile_delete": {
        const p = profiles.find((o) => o.id === args.id);
        if (!p) throw new EngineError("not_found", "That profile no longer exists.");
        profiles = profiles.filter((o) => o.id !== args.id);
        if (active?.id === p.id) active = null;
        activity(`Removed profile ${p.name}`, started);
        return null;
      }
      case "profile_test": {
        const p = profiles.find((o) => o.id === args.id);
        if (!p) throw new EngineError("not_found", "That profile no longer exists.");
        activity(`Tested ${p.name}: management ok, messaging ok`, started);
        return {
          host: p.host, emulator: false, transport: p.transport, auth: "SAS key reader",
          management: { status: "ok", detail: "listed 4 queue(s) (Manage)" },
          messaging: { status: "ok", detail: "peeked customs-docs (Listen)" },
        };
      }
      case "account_current":
        return account;
      case "sign_in": {
        emit("sign-in-prompt", args.deviceCode
          ? { kind: "device-code", url: "https://microsoft.com/devicelogin", user_code: "QX4T-9KLM", message: "Open the page and enter the code." }
          : { kind: "browser", url: "https://login.microsoftonline.com/", user_code: null, message: "Finish signing in in your browser." });
        await wait(400);
        account = { id: "oid-1", user_principal: "ops@contoso.example", display_name: "Ops", tenant_id: "tenant-1" };
        activity(`Signed in as ${account.user_principal}`, started);
        return account;
      }
      case "sign_in_cancel":
        return null;
      case "connect": {
        const p = profiles.find((o) => o.id === args.profileId);
        if (!p) throw new EngineError("not_found", "That profile no longer exists.");
        if (scenario === "missing-secret") {
          const e = new EngineError("invalid", `profile \`${p.name}\` has no stored connection string; remove and add it again`);
          activity(`Connecting to ${p.name} failed`, started, e);
          throw e;
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
        if (!active) throw new EngineError("not_connected", "Choose a connection first.");
        if (scenario === "network") {
          const e = new EngineError("network", "network error: connection reset by peer");
          activity("Listing the namespace failed", started, e);
          throw e;
        }
        if (scenario === "forbidden") {
          activity("Listing not allowed; showing 2 pinned entities", started);
          return {
            queues: [], topics: [],
            limited: "This identity can't list the namespace (needs Manage,EntityRead). Showing pinned entities.",
            known: [{ entity: "orders", subscription: null }, { entity: "shipment-events", subscription: "tracking-sync" }],
          } satisfies EntityListing;
        }
        const queues = scenario === "many" ? fx.manyQueues : fx.queues;
        const topics = scenario === "many-topics" ? fx.manyTopics : fx.topics;
        activity(`Listed ${queues.length} queues and ${topics.length} topics`, started);
        return { queues, topics, limited: null, known: [] } satisfies EntityListing;
      }
      case "list_subscriptions": {
        if (!active) throw new EngineError("not_connected", "Choose a connection first.");
        const subs = fx.subscriptions[args.topic as string] ?? [];
        activity(`Listed ${subs.length} subscriptions of ${args.topic}`, started);
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

/** Mock builds: `?scenario=` picks the scenario, `?autoconnect=<profile id>`
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
  setTransport(createMockTransport(scenario, 120, autoconnect ? { ...seed, last_profile: autoconnect } : seed));
}
