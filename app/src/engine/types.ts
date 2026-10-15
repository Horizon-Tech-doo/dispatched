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
  | { type: "started"; id: string; title: string; total: number | null }
  | { type: "progress"; id: string; done: number; total: number | null }
  | { type: "finished"; id: string; error: ActivityError | null };

export interface AppInfo { version: string; startup_error: string | null }
