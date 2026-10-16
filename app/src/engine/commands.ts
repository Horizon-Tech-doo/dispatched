import { localOffsetMinutes } from "./exportName";
import { call } from "./ipc";
import type {
  AccountView, AppInfo, ConnectionReport, ConnectionView, DashboardView, DiscoveryView, EntityDetails, EntityKindName, EntityListing, ExportFormat,
  ExportRequest, FilterIssue, FilterSpec, JobStarted, MessageDetail, PeekPageView, PeekRequest, PeekView, PickedPath, Profile, ProfileDraft, RuleInfo,
  SessionListing, SignedOut, SignInView, StoredSignIn, SubscriptionInfo, TenantView,
} from "./types";

export const appInfo = () => call<AppInfo>("app_info");
export const settingsGet = (key: string) => call<unknown>("settings_get", { key });
export const settingsSet = (key: string, value: unknown) => call<void>("settings_set", { key, value });

export const profilesList = () => call<Profile[]>("profiles_list");
export const profileSave = (draft: ProfileDraft) => call<Profile>("profile_save", { draft });
export const profileDelete = (id: string) => call<void>("profile_delete", { id });
export const profileTest = (id: string) => call<ConnectionReport>("profile_test", { id });

export const accountCurrent = () => call<AccountView | null>("account_current");
/** Signs in. With a tenant other than the current account's home, the sign-in is that tenant's own and the account stays. */
export const signIn = (tenant: string | null, deviceCode: boolean) => call<SignInView>("sign_in", { tenant, deviceCode });
export const signInCancel = () => call<void>("sign_in_cancel");

/** The signed-in account's tenants, home first. */
export const listTenants = () => call<TenantView[]>("list_tenants");
/** Service Bus namespaces in one tenant (`null` is the home tenant). A guest tenant that needs multi-factor sign-in fails with kind `mfa_required`. */
export const discoverNamespaces = (tenant: string | null) => call<DiscoveryView>("discover_namespaces", { tenant });

export const connect = (profileId: string) => call<ConnectionView>("connect", { profileId });
export const disconnect = () => call<void>("disconnect");

/** Listings name the profile they are for; the engine refuses one that is no longer connected. */
export const listEntities = (profileId: string) => call<EntityListing>("list_entities", { profileId });
export const listSubscriptions = (profileId: string, topic: string) => call<SubscriptionInfo[]>("list_subscriptions", { profileId, topic });

/** One page of summaries; the engine caps a page at 500 (from the end: 5,000) and its scan at 5,000 (100,000). */
export const peekPage = (profileId: string, request: PeekRequest) => call<PeekPageView>("peek_page", { profileId, request });
/** The full message (body capped at 4 MiB), or `null` when it is no longer in the view. `sessionId` is the session the
 *  row was peeked in (a per-session view): the engine reads it through that session. */
export const messageDetail = (profileId: string, entity: string, view: PeekView, sequence: number, sessionId: string | null = null) =>
  call<MessageDetail | null>("message_detail", { profileId, entity, view, sequence, sessionId });
/** Problems with a filter, at most one per field; empty when it compiles. Local: no connection needed. */
export const checkFilter = (filter: FilterSpec) => call<FilterIssue[]>("check_filter", { filter });
export const entityDetails = (profileId: string, kind: EntityKindName, entity: string) => call<EntityDetails>("entity_details", { profileId, kind, entity });
/** `entity` is `topic/subscriptions/name`; anything else is refused with `invalid`. */
export const listRules = (profileId: string, entity: string) => call<RuleInfo[]>("list_rules", { profileId, entity });
export const listSessions = (profileId: string, entity: string) => call<SessionListing>("list_sessions", { profileId, entity });
export const dashboard = (profileId: string) => call<DashboardView>("dashboard", { profileId });
/** The native save dialog, with a suggested file name; `null` when the user cancels it. The path stays in the
 *  engine under the returned one-time `pick`, which `exportStart` takes. */
/** The save dialog. `localOffsetMinutes` lets the engine stamp the suggested name in the user's local time. */
export const pickExportPath = (entity: string, view: PeekView, format: ExportFormat) =>
  call<PickedPath | null>("pick_export_path", { entity, view, format, localOffsetMinutes: localOffsetMinutes() });
/** Starts an export job (events on `onJob`). An unknown or spent pick fails with `invalid`
 *  "Choose where to save the export."; a request refused before the job starts keeps its pick. */
export const exportStart = (profileId: string, request: ExportRequest) => call<JobStarted>("export_start", { profileId, request });
/** False when there is no such running job. */
export const jobCancel = (id: string) => call<boolean>("job_cancel", { id });
/** Shows a finished export's file in Finder or Explorer (Linux: its folder). */
export const jobReveal = (id: string) => call<void>("job_reveal", { id });
/** The draft may carry a connection string: it is used for this test only. */
export const profileTestDraft = (draft: ProfileDraft) => call<ConnectionReport>("profile_test_draft", { draft });
/** Rust writes the clipboard; the string never comes back. Refused for Entra profiles and unsafe entity names. */
export const copyEntityConnectionString = (profileId: string, entity: string) => call<void>("copy_entity_connection_string", { profileId, entity });
export const signInsList = () => call<StoredSignIn[]>("sign_ins_list");
/** Forgets a sign-in; closes the active connection when it used it (`closed_profile_id`). */
export const signOut = (id: string) => call<SignedOut>("sign_out", { id });
/** Opens the waiting sign-in's https page again; `not_found` when no sign-in is waiting. */
export const openSignInPage = () => call<void>("open_sign_in_page");
