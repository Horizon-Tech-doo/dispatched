// Mirrors the Rust types that cross IPC (snake_case fields, as serde writes them).

export type Transport = "amqp" | "websockets";
export type ProfileAuth = { kind: "connection-string" } | { kind: "entra"; sign_in_id: string; tenant: string | null };

/** The sign-in an Entra profile uses now, as the engine sees it. */
export interface BoundSignIn {
  id: string;
  user_principal: string;
  /** The sign-in's own (home) tenant. */
  tenant_id: string;
  /** Its refresh token exists: connecting works without signing in again. */
  live: boolean;
  /** It is the current account's identity. */
  same_account: boolean;
  /** It is a tenant's own sign-in, outside the current account's home tenant: the profile's tenant is
   *  `tenant_id` even when `auth.tenant` is null. */
  own_tenant: boolean;
}

/** A profile as saved, without what the engine works out about it. */
export interface StoredProfile {
  id: string;
  name: string;
  host: string;
  auth: ProfileAuth;
  emulator: boolean;
  production: boolean;
  color: string | null;
  transport: Transport;
  proxy: string | null;
  pinned_entities: string[];
}

export interface Profile extends StoredProfile {
  /** For Entra profiles: the sign-in used, or null if its record is gone. */
  bound_sign_in: BoundSignIn | null;
}

export type DraftAuth =
  | { kind: "connection-string"; connection_string?: string | null }
  | { kind: "entra"; namespace: string; tenant?: string | null };

export interface ProfileDraft {
  id?: string | null;
  name: string;
  auth: DraftAuth;
  production: boolean;
  color?: string | null;
  transport: Transport;
  proxy?: string | null;
  pinned_entities: string[];
}

export type Check =
  | { status: "ok"; detail: string }
  | { status: "failed"; kind: string; message: string }
  | { status: "skipped"; reason: string };

export interface ConnectionReport {
  host: string;
  emulator: boolean;
  transport: Transport;
  auth: string;
  management: Check;
  messaging: Check;
}

export interface AccountView { id: string; user_principal: string; display_name: string | null; tenant_id: string }
/** A sign-in made in a tenant that did not change the current account. */
export interface TenantSignIn { tenant_id: string; user_principal: string; same_account: boolean }
export interface SignInView { account: AccountView; tenant_sign_in: TenantSignIn | null }
export interface SignInPrompt { kind: "browser" | "device-code"; url: string; user_code: string | null; message: string }

/** A tenant the signed-in account can reach. `home` is the one the account signed in to; the list puts it first. */
export interface TenantView {
  tenant_id: string;
  display_name: string | null;
  default_domain: string | null;
  home: boolean;
  /** The current account's own sign-in in this tenant, when the tenant is listed with it. */
  sign_in_upn: string | null;
}

export interface DiscoveredNamespace {
  name: string;
  /** Full host, like `sb-orders.servicebus.windows.net`. */
  host: string;
  resource_id: string;
  subscription_id: string;
  subscription_name: string;
  tenant_id: string | null;
  location: string | null;
  tier: string | null;
  /** Shared access keys are turned off: Entra ID only. */
  local_auth_disabled: boolean;
}

export interface SkippedSubscription { subscription_id: string; subscription_name: string; reason: string }
export interface DiscoveryView { namespaces: DiscoveredNamespace[]; skipped: SkippedSubscription[] }

export interface ConnectionView {
  profile_id: string;
  profile_name: string;
  host: string;
  emulator: boolean;
  auth: string;
  transport: Transport;
  production: boolean;
}

export interface EntityCounts {
  active: number;
  dead_letter: number;
  scheduled: number;
  transfer: number;
  transfer_dead_letter: number;
  approximate: boolean;
  truncated: boolean;
}

export interface QueueInfo {
  name: string;
  status: string | null;
  counts: EntityCounts | null;
  size_in_bytes: number | null;
  requires_session: boolean;
  max_delivery_count: number | null;
  lock_duration: string | null;
  forward_to: string | null;
}

export interface TopicInfo { name: string; status: string | null; size_in_bytes: number | null; subscription_count: number | null }

export interface SubscriptionInfo {
  topic: string;
  name: string;
  status: string | null;
  counts: EntityCounts | null;
  requires_session: boolean;
  max_delivery_count: number | null;
  forward_to: string | null;
}

export interface EntityRef { entity: string; subscription: string | null }

export interface EntityListing { queues: QueueInfo[]; topics: TopicInfo[]; limited: string | null; known: EntityRef[] }

export interface ActivityError { kind: string; message: string }
export interface ActivityEntry { at: string; action: string; duration_ms: number; error: ActivityError | null }

export type JobEvent =
  /** `title` says what the job is (`Export orders (dead-letter queue)`); `file_name` is the file it writes. */
  | { type: "started"; id: string; title: string; total: number | null; file_name: string | null }
  /** `scanned` is the messages examined so far: past `done` when a filter skips messages. */
  | { type: "progress"; id: string; done: number; total: number | null; scanned: number }
  | {
      type: "finished";
      id: string;
      error: ActivityError | null;
      /** What was done, e.g. `Exported 1,284 messages to orders-dead-letter-20261016-0941.json`; null on failure. */
      summary: string | null;
      /** The file written, in full; null on failure. */
      output: string | null;
      /** Messages written; null on failure. */
      written: number | null;
      /** Selected messages that were no longer there; null on failure. */
      missing: number | null;
      /** The file written, by name; null on failure. */
      file_name: string | null;
    };

export interface AppInfo { version: string; startup_error: string | null }

// M1c: browse, entity, dashboard, export and sign-ins.

export type PeekView = "active" | "scheduled" | "deferred" | "dead-letter" | "transfer-dead-letter";
export type MessageState = "active" | "deferred" | "scheduled";
/** An application property; `binary` values are base64, `timestamp` RFC 3339, `timespan` ISO 8601 duration.
 *  Numbers keep their AMQP type: `byte`, `short`, `int`, `long`, `ubyte`, `ushort`, `uint`, `ulong`, `float`, `double`. */
export type PropValue =
  | { type: "null" }
  | { type: "bool"; value: boolean }
  | { type: "byte" | "short" | "int" | "long" | "ubyte" | "ushort" | "uint" | "ulong" | "float" | "double"; value: number }
  | { type: "string" | "symbol" | "timestamp" | "timespan" | "uuid" | "binary" | "other"; value: string };
/** `binary` values are base64. `value` and `sequence` bodies are AMQP values rendered as JSON. */
export type MessageBody =
  | { kind: "binary"; value: string }
  | { kind: "text"; value: string }
  | { kind: "value" | "sequence"; value: unknown }
  | { kind: "empty" };

interface MessageFields {
  sequence_number: number;
  state: MessageState;
  message_id: string | null;
  correlation_id: string | null;
  session_id: string | null;
  subject: string | null;
  content_type: string | null;
  to: string | null;
  reply_to: string | null;
  reply_to_session_id: string | null;
  partition_key: string | null;
  /** When the message expires: the broker's absolute-expiry-time, else enqueued time plus time to live. */
  expires_at: string | null;
  enqueued_time: string | null;
  scheduled_enqueue_time: string | null;
  delivery_count: number;
  time_to_live_ms: number | null;
  dead_letter_reason: string | null;
  dead_letter_description: string | null;
  dead_letter_source: string | null;
  application_properties: Record<string, PropValue>;
}

/** A grid row: no body, a 160-character preview (null for empty and non-text bodies). */
export interface MessageSummary extends MessageFields {
  body_kind: MessageBody["kind"];
  body_size: number;
  body_preview: string | null;
}

/** One message; bodies above 4 MiB arrive cut, with `body_truncated` (`body_size` is the full size). */
export interface MessageDetail extends MessageFields {
  locked_until: string | null;
  body: MessageBody;
  body_size: number;
  body_truncated: boolean;
}

/** Filter fields as typed. A field that is blank after a trim means "no condition"; otherwise the text is
 *  used as typed (a regex or expected value keeps its spaces). */
export interface FilterSpec {
  property?: string | null;
  body_contains?: string | null;
  body_regex?: string | null;
  body_json_path?: string | null;
  body_json_equals?: string | null;
  session_id?: string | null;
}
export type FilterField = "property" | "body_regex" | "body_json_path" | "body_json_equals";
export interface FilterIssue { field: FilterField; message: string }

export type PeekFrom = { kind: "start" } | { kind: "sequence"; sequence: number } | { kind: "end" };
export interface PeekRequest { entity: string; view: PeekView; from: PeekFrom; max: number; scan_limit?: number | null; filter: FilterSpec }
/** One page of summaries.
 *  - Forward pages continue at `next_sequence`; `exhausted` means the end of the view was reached.
 *  - "From the end" pages have `next_sequence: null`. `exhausted: false` means the budget ran out inside the
 *    newest window, so the rows are not the newest. With `exhausted: true`, fewer rows than asked and
 *    `scanned >= scan_limit`, older messages may not have been examined. */
export interface PeekPageView {
  messages: MessageSummary[];
  /** Messages this call examined. */
  scanned: number;
  /** The scan limit this call ran with, after clamping. */
  scan_limit: number;
  next_sequence: number | null;
  exhausted: boolean;
}

export type EntityKindName = "queue" | "topic" | "subscription";
export interface EntityProperty { name: string; value: string }
export interface EntityDetails {
  kind: EntityKindName;
  /** The subscription name for a subscription. */
  name: string;
  topic: string | null;
  status: string | null;
  /** Null when not reported (topics, the emulator) or when reading them failed. */
  counts: EntityCounts | null;
  size_in_bytes: number | null;
  properties: EntityProperty[];
}

export interface CorrelationProperty { name: string; value: string }
export type RuleFilter =
  | { kind: "sql"; expression: string }
  | {
      kind: "correlation";
      correlation_id: string | null;
      message_id: string | null;
      to: string | null;
      reply_to: string | null;
      subject: string | null;
      session_id: string | null;
      reply_to_session_id: string | null;
      content_type: string | null;
      properties: CorrelationProperty[];
    }
  /** A filter the reader does not understand: shown as "Unknown filter", with no fields. */
  | { kind: "unknown" };
export interface RuleInfo { name: string; filter: RuleFilter; action: string | null; created_at: string | null }

export interface SessionSummary {
  id: string;
  /** Messages the tally saw; null when no tally ran. */
  messages: number | null;
  first_enqueued: string | null;
  last_enqueued: string | null;
  /** The broker listed this id. */
  in_broker: boolean;
}
/** What the broker's session list contributed: `listed` (cut at 10,000 when `capped`), `empty` (the sessions
 *  come from the tally), or `unavailable` (the call failed with `kind`; the sessions come from peeking). */
export type BrokerList = { state: "listed"; capped: boolean } | { state: "empty" } | { state: "unavailable"; kind: string };
export interface SessionListing {
  /** Sorted by id. */
  sessions: SessionSummary[];
  broker: BrokerList;
  /** Messages the tally examined. */
  scanned: number;
  /** The tally hit its cap: message counts are lower bounds, and the session set may be incomplete when the
   *  broker list is `empty` or `unavailable`. */
  truncated: boolean;
}

export interface DashboardRow {
  kind: "queue" | "subscription";
  /** The entity's tab id: a queue name or `topic/subscriptions/name`. */
  id: string;
  name: string;
  topic: string | null;
  status: string | null;
  counts: EntityCounts | null;
}
export interface DashboardFailure { topic: string; kind: string; message: string }
/** `limited` explains why only known entities show (without counts). In that view a topic named by the
 *  connection string's `EntityPath` comes back as a queue row. */
export interface DashboardView { rows: DashboardRow[]; failures: DashboardFailure[]; limited: string | null }

export type ExportFormat = "json" | "ndjson";
export type ExportScope =
  | { kind: "all"; filter: FilterSpec; from_sequence?: number; max?: number | null }
  /** `session_id`: the session the rows were peeked in (a per-session view); they are read again through it. */
  | { kind: "selected"; sequences: number[]; session_id?: string | null };
/** `pick` is the one-time id `pickExportPath` returned; the webview never sends a path. Unknown fields
 *  (`path`, `overwrite`) are refused. */
export interface ExportRequest { entity: string; view: PeekView; scope: ExportScope; format: ExportFormat; pick: string; total_hint?: number | null }
/** A path the save dialog returned, kept in the engine under `pick`. `name` and `dir` are for display only. */
export interface PickedPath { pick: string; name: string; dir: string }
export interface JobStarted { id: string }

/** One remembered sign-in in Settings (Rust `StoredSignIn`). `SignInView` stays what `signIn` returns. */
export interface StoredSignIn {
  id: string;
  user_principal: string;
  display_name: string | null;
  tenant_id: string;
  /** The current account's sign-in. */
  current: boolean;
  /** Its refresh token exists. */
  signed_in: boolean;
  /** The current account's identity; another account's kept tenant sign-in is not. */
  same_account: boolean;
  /** Names of the profiles that connect with this sign-in now (`bound_sign_in`). */
  profiles: string[];
}
/** `closed_profile_id`: the active connection used the sign-in and was closed (its jobs cancelled). */
export interface SignedOut { closed_profile_id: string | null }
