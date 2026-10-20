import { localOffsetMinutes } from "./exportName";
import { call } from "./ipc";
import type {
  ActivityQuery, ActivityRecord, ConfirmAnswer, DlqSummaryView, ImportForm, OpenedFiles, OpenPurpose, PurgePlans, PurgeScope, ReceiveForm,
  ReportPage, ResubmitForm, SelectForm, SendForm, SessionStateView, Settlement, SubQueue,
  AccountView, AppInfo, ConnectionReport, ConnectionView, DashboardView, DiscoveryView, EntityDetails, EntityKindName, EntityListing, ExportFormat,
  ExportRequest, FilterIssue, FilterSpec, JobStarted, MessageDetail, PeekPageView, PeekRequest, PeekView, PickedPath, Profile, ProfileDraft, RuleInfo,
  SessionListing, SignedOut, SignInView, StoredSignIn, SubscriptionInfo, TenantView,
  EntityRefForm, EntityStatus, ExportedEntities, FieldIssue, ImportPreview, KeySlot, ListenForm, PolicyChange, PolicySecret, PolicyView, RuleForm,
  SettingsChange, SettingsView,
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

// ---- M2b operate: each `op*` returns the job id at once; confirmations, items and the finish arrive as events ----

export const opSend = (profileId: string, form: SendForm) => call<JobStarted>("op_send", { profileId, form });
export const opImport = (profileId: string, form: ImportForm) => call<JobStarted>("op_import", { profileId, form });
/** Sequence numbers as strings (they can exceed 2^53). */
export const opCancelScheduled = (profileId: string, entity: string, sequences: string[]) =>
  call<JobStarted>("op_cancel_scheduled", { profileId, entity, sequences });
export const opReceive = (profileId: string, form: ReceiveForm) => call<JobStarted>("op_receive", { profileId, form });
export const opSettleDeferred = (profileId: string, entity: string, subQueue: SubQueue, sequences: string[], session: string | null, how: Settlement) =>
  call<JobStarted>("op_settle_deferred", { profileId, entity, subQueue, sequences, session, how });
export const opDelete = (profileId: string, form: SelectForm) => call<JobStarted>("op_delete", { profileId, form });
export const opMove = (profileId: string, form: SelectForm, target: string, dropUncopyable: boolean) =>
  call<JobStarted>("op_move", { profileId, form, target, dropUncopyable });
export const opResubmit = (profileId: string, form: ResubmitForm) => call<JobStarted>("op_resubmit", { profileId, form });
/** Counts what a purge would remove (nothing changes). The plans run once, within 10 minutes, through `opPurge`. */
export const purgePlan = (profileId: string, entities: string[], scope: PurgeScope) => call<PurgePlans>("purge_plan", { profileId, entities, scope });
export const opPurge = (profileId: string, planId: string) => call<JobStarted>("op_purge", { profileId, planId });
export const dlqSummary = (profileId: string, entity: string, subQueue: SubQueue) => call<DlqSummaryView>("dlq_summary", { profileId, entity, subQueue });
/** `null` when the session has no state. */
export const sessionStateGet = (profileId: string, entity: string, session: string) =>
  call<SessionStateView | null>("session_state_get", { profileId, entity, session });
/** `value` is base64, or null to clear it. Over 256 KiB needs `allowLarge`. */
export const opSessionStateSet = (profileId: string, entity: string, session: string, value: string | null, allowLarge: boolean) =>
  call<JobStarted>("op_session_state_set", { profileId, entity, session, value, allowLarge });
/** Save the session's current state (read fresh) to a file the native save dialog picks; the file name, or null
 *  when cancelled or there is no state. */
export const saveSessionState = (profileId: string, entity: string, session: string) =>
  call<string | null>("save_session_state", { profileId, entity, session });
/** What a pinned or `EntityPath` entity is, or null when the namespace does not say. */
export const resolveKind = (profileId: string, entity: string) => call<EntityKindName | null>("resolve_kind", { profileId, entity });
/** The native open dialog; `null` when cancelled. The paths stay in the engine under `pick`. */
export const pickOpenPaths = (purpose: OpenPurpose) => call<OpenedFiles | null>("pick_open_paths", { purpose });
/** A page of a finished job's items (`limit` at most 1,000); null when the job has no report. */
export const jobReport = (id: string, offset: number, limit: number, onlyProblems: boolean) =>
  call<ReportPage | null>("job_report", { id, offset, limit, onlyProblems });
export const pickReportPath = (id: string, format: ExportFormat) => call<PickedPath | null>("pick_report_path", { id, format });
/** Writes the report; returns the path written. */
export const jobReportSave = (id: string, pick: string, format: ExportFormat) => call<string>("job_report_save", { id, pick, format });
/** False when the confirmation no longer waits. */
export const confirmAnswer = (id: string, answer: ConfirmAnswer) => call<boolean>("confirm_answer", { id, answer });
/** The shared activity log, newest first. */
export const activityList = (query: ActivityQuery) => call<ActivityRecord[]>("activity_list", { query });
/** Entries after row `afterId`, oldest first; `null` returns just the newest entry, to follow from it. */
export const activitySince = (afterId: number | null) => call<ActivityRecord[]>("activity_since", { afterId });
export const pickActivityExportPath = (format: ExportFormat) => call<PickedPath | null>("pick_activity_export_path", { format });
/** Writes the matching entries; returns how many. */
export const activityExport = (query: ActivityQuery, pick: string, format: ExportFormat) => call<number>("activity_export", { query, pick, format });

// ---- M3b manage: reads, checks, and `op*` jobs (the job id comes back at once; confirmations and the report are events) ----

/** The entity's settings, read fresh, for its edit form. */
export const entitySettings = (profileId: string, kind: EntityKindName, entity: string) => call<SettingsView>("entity_settings", { profileId, kind, entity });
/** Names and rights; the keys never leave the engine. Subscriptions have none (`invalid`). */
export const entityPolicies = (profileId: string, kind: EntityKindName, entity: string) => call<PolicyView[]>("entity_policies", { profileId, kind, entity });
/** Problems with a settings form, one per field; local. */
export const checkSettings = (kind: EntityKindName, form: SettingsChange, create: boolean) => call<FieldIssue[]>("check_settings", { kind, form, create });
/** Problems with a rule (`name`, `filter`); local. */
export const checkRule = (rule: RuleForm) => call<FieldIssue[]>("check_rule", { rule });
/** Rust writes the key or connection string to the clipboard; nothing comes back. */
export const copyPolicySecret = (profileId: string, kind: EntityKindName, entity: string, policy: string, slot: KeySlot, what: PolicySecret) =>
  call<void>("copy_policy_secret", { profileId, kind, entity, policy, slot, what });
export const opEntityCreate = (profileId: string, kind: EntityKindName, entity: string, settings: SettingsChange, rule: RuleForm | null) =>
  call<JobStarted>("op_entity_create", { profileId, kind, entity, settings, rule });
/** Only `changes` are applied, to the entity as it is when the job runs. */
export const opEntityUpdate = (profileId: string, kind: EntityKindName, entity: string, changes: SettingsChange) =>
  call<JobStarted>("op_entity_update", { profileId, kind, entity, changes });
export const opEntityDelete = (profileId: string, entities: EntityRefForm[]) => call<JobStarted>("op_entity_delete", { profileId, entities });
export const opEntityStatus = (profileId: string, target: EntityRefForm, status: EntityStatus) => call<JobStarted>("op_entity_status", { profileId, target, status });
/** Duplicate, or with `rename` rename (create the copy, then delete the original). */
export const opEntityCopy = (profileId: string, target: EntityRefForm, newName: string, rename: boolean) =>
  call<JobStarted>("op_entity_copy", { profileId, target, newName, rename });
/** Add a rule, or with `update` replace an existing one's filter and action. */
export const opRuleSave = (profileId: string, entity: string, rule: RuleForm, update: boolean) => call<JobStarted>("op_rule_save", { profileId, entity, rule, update });
export const opRuleRemove = (profileId: string, entity: string, name: string) => call<JobStarted>("op_rule_remove", { profileId, entity, name });
export const opPolicyChange = (profileId: string, target: EntityRefForm, name: string, change: PolicyChange) =>
  call<JobStarted>("op_policy_change", { profileId, target, name, change });
export const pickEntitiesExportPath = (namespace: string) =>
  call<PickedPath | null>("pick_entities_export_path", { namespace, localOffsetMinutes: localOffsetMinutes() });
/** Writes the ServiceBusExplorer XML of `entities` (none: the whole namespace). A read: nothing is confirmed. */
export const entitiesExport = (profileId: string, entities: EntityRefForm[], includeKeys: boolean, pick: string) =>
  call<ExportedEntities>("entities_export", { profileId, entities, includeKeys, pick });
/** Reads and checks the file picked with purpose `entities`; nothing changes yet. */
export const entitiesImportPreview = (profileId: string, pick: string) => call<ImportPreview>("entities_import_preview", { profileId, pick });
export const opEntitiesImport = (profileId: string, previewId: string, overwrite: boolean) => call<JobStarted>("op_entities_import", { profileId, previewId, overwrite });
/** Starts a listener; it runs until `jobCancel`. Each message arrives as a `received` job event. */
export const opListen = (profileId: string, form: ListenForm) => call<JobStarted>("op_listen", { profileId, form });
/** A message a listener or receive job took, by sequence number (text); null when it holds none. */
export const jobMessage = (id: string, sequence: string) => call<MessageDetail | null>("job_message", { id, sequence });
