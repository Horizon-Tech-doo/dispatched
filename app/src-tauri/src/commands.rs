//! Tauri commands: unwrap state and handle, call `ops`.

use crate::error::CommandError;
use crate::events::TauriEvents;
use crate::ops;
use crate::ops::profiles::ProfileView;
use crate::state::AppState;
use dispatched_core::diagnostics::ConnectionReport;
use dispatched_core::profiles::ProfileDraft;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, State};

#[derive(Serialize)]
pub struct AppInfo {
    version: String,
    startup_error: Option<String>,
}

#[tauri::command]
pub fn app_info(app: AppHandle, state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        startup_error: state.startup_error.clone(),
    }
}

// Keychain and SQLite calls: async commands run off the main thread.
#[tauri::command]
pub async fn settings_get(
    state: State<'_, AppState>,
    key: String,
) -> Result<Option<Value>, CommandError> {
    ops::settings::get(&state, &key)
}

#[tauri::command]
pub async fn settings_set(
    state: State<'_, AppState>,
    key: String,
    value: Option<Value>,
) -> Result<(), CommandError> {
    ops::settings::set(&state, &key, value)
}

#[tauri::command]
pub async fn profiles_list(state: State<'_, AppState>) -> Result<Vec<ProfileView>, CommandError> {
    ops::profiles::list(&state)
}

#[tauri::command]
pub async fn profile_save(
    app: AppHandle,
    state: State<'_, AppState>,
    draft: ProfileDraft,
) -> Result<ProfileView, CommandError> {
    ops::profiles::save(&state, &TauriEvents(app), draft).await
}

#[tauri::command]
pub async fn profile_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), CommandError> {
    ops::profiles::delete(&state, &TauriEvents(app), &id).await
}

#[tauri::command]
pub async fn profile_test(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<ConnectionReport, CommandError> {
    ops::profiles::test(&state, &TauriEvents(app), &id).await
}

#[tauri::command]
pub async fn profile_test_draft(
    app: AppHandle,
    state: State<'_, AppState>,
    draft: ProfileDraft,
) -> Result<ConnectionReport, CommandError> {
    ops::profiles::test_draft(&state, &TauriEvents(app), draft).await
}

/// Writes the entity's connection string to the clipboard from Rust; it is never returned.
#[tauri::command]
pub async fn copy_entity_connection_string(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    entity: String,
) -> Result<(), CommandError> {
    ops::profiles::copy_entity_connection_string(
        &state,
        &TauriEvents(app),
        &profile_id,
        &entity,
        crate::clipboard::write_secret_text,
    )
    .await
}

use crate::ops::account::{AccountView, SignInView, SignedOut, StoredSignIn};
use crate::ops::browse::{PeekPageView, PeekRequest};
use crate::ops::connection::ConnectionView;
use crate::ops::dashboard::DashboardView;
use crate::ops::discovery::{DiscoveryView, TenantView};
use crate::ops::export::{ExportRequest, JobStarted, PickedPath};
use crate::ops::tree::EntityListing;
use dispatched_core::{
    EntityDetails, EntityKind, FilterIssue, FilterSpec, MessageDetail, PeekView, RuleInfo,
    SessionListing, SubscriptionInfo,
};

#[tauri::command]
pub async fn account_current(
    state: State<'_, AppState>,
) -> Result<Option<AccountView>, CommandError> {
    ops::account::current(&state)
}

#[tauri::command]
pub async fn sign_in(
    app: AppHandle,
    state: State<'_, AppState>,
    tenant: Option<String>,
    device_code: bool,
) -> Result<SignInView, CommandError> {
    ops::account::sign_in(&state, &TauriEvents(app), tenant, device_code).await
}

#[tauri::command]
pub fn sign_in_cancel(state: State<'_, AppState>) {
    ops::account::cancel(&state);
}

#[tauri::command]
pub async fn sign_ins_list(state: State<'_, AppState>) -> Result<Vec<StoredSignIn>, CommandError> {
    ops::account::sign_ins(&state)
}

#[tauri::command]
pub async fn sign_out(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<SignedOut, CommandError> {
    ops::account::sign_out(&state, &TauriEvents(app), &id).await
}

#[tauri::command]
pub fn open_sign_in_page(state: State<'_, AppState>) -> Result<(), CommandError> {
    ops::account::open_sign_in_page(&state)
}

#[tauri::command]
pub async fn list_tenants(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<TenantView>, CommandError> {
    ops::discovery::list_tenants(&state, &TauriEvents(app)).await
}

#[tauri::command]
pub async fn discover_namespaces(
    app: AppHandle,
    state: State<'_, AppState>,
    tenant: Option<String>,
) -> Result<DiscoveryView, CommandError> {
    ops::discovery::discover(&state, &TauriEvents(app), tenant).await
}

#[tauri::command]
pub async fn connect(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<ConnectionView, CommandError> {
    ops::connection::connect(&state, &TauriEvents(app), &profile_id).await
}

#[tauri::command]
pub async fn disconnect(app: AppHandle, state: State<'_, AppState>) -> Result<(), CommandError> {
    ops::connection::disconnect(&state, &TauriEvents(app)).await
}

#[tauri::command]
pub async fn list_entities(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<EntityListing, CommandError> {
    ops::tree::list_entities(&state, &TauriEvents(app), &profile_id).await
}

#[tauri::command]
pub async fn list_subscriptions(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    topic: String,
) -> Result<Vec<SubscriptionInfo>, CommandError> {
    ops::tree::list_subscriptions(&state, &TauriEvents(app), &profile_id, &topic).await
}

#[tauri::command]
pub async fn peek_page(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    request: PeekRequest,
) -> Result<PeekPageView, CommandError> {
    ops::browse::peek_page(&state, &TauriEvents(app), &profile_id, request).await
}

#[tauri::command]
pub async fn message_detail(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    entity: String,
    view: PeekView,
    sequence: i64,
    session_id: Option<String>,
) -> Result<Option<MessageDetail>, CommandError> {
    ops::browse::message_detail(
        &state,
        &TauriEvents(app),
        &profile_id,
        &entity,
        view,
        sequence,
        session_id.as_deref(),
    )
    .await
}

/// Pure and local: no network, no state.
#[tauri::command]
pub fn check_filter(filter: FilterSpec) -> Vec<FilterIssue> {
    ops::browse::check_filter(&filter)
}

#[tauri::command]
pub async fn entity_details(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    kind: EntityKind,
    entity: String,
) -> Result<EntityDetails, CommandError> {
    ops::entity::entity_details(&state, &TauriEvents(app), &profile_id, kind, &entity).await
}

/// `entity` is `topic/subscriptions/name`.
#[tauri::command]
pub async fn list_rules(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    entity: String,
) -> Result<Vec<RuleInfo>, CommandError> {
    ops::entity::list_rules(&state, &TauriEvents(app), &profile_id, &entity).await
}

#[tauri::command]
pub async fn list_sessions(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    entity: String,
) -> Result<SessionListing, CommandError> {
    ops::entity::list_sessions(&state, &TauriEvents(app), &profile_id, &entity).await
}

#[tauri::command]
pub async fn dashboard(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<DashboardView, CommandError> {
    ops::dashboard::dashboard(&state, &TauriEvents(app), &profile_id).await
}

/// The native save dialog for an export, with a suggested file name, as a sheet on the calling
/// window. The chosen path stays in Rust under a one-time pick id; `export_start` takes the id,
/// so an export can only write where the user chose (the dialog asked "replace?" there).
/// Returns `None` when the dialog was cancelled.
#[tauri::command]
pub async fn pick_export_path(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    entity: String,
    view: PeekView,
    format: String,
    local_offset_minutes: Option<i32>,
) -> Result<Option<PickedPath>, CommandError> {
    use tauri_plugin_dialog::DialogExt;
    // The user's clock: `time` can't read the local offset soundly in a multi-threaded process on
    // Unix, so the webview says it. Without it, the name is UTC and marked `Z`.
    let now = ops::export::stamp_time(time::OffsetDateTime::now_utc(), local_offset_minutes);
    let name = ops::export::suggested_file_name(&entity, view, &format, now);
    let (label, ext) = if format.eq_ignore_ascii_case("ndjson") {
        ("NDJSON", "ndjson")
    } else {
        ("JSON", "json")
    };
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_parent(&window)
            .set_file_name(&name)
            .add_filter(label, &[ext])
            .blocking_save_file()
    })
    .await
    .map_err(|e| CommandError::new("io", format!("The save dialog failed: {e}")))?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|path| ops::export::remember_pick(&state.jobs, path)))
}

#[tauri::command]
pub async fn export_start(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    request: ExportRequest,
) -> Result<JobStarted, CommandError> {
    // Held while the job is registered, so a connect or disconnect that closes this explorer
    // either comes first (then `active_for` refuses) or cancels the job.
    let _serial = state.connect_lock.lock().await;
    let active = state.active_for(&profile_id).await?;
    ops::export::start(
        &state.jobs,
        active.explorer,
        std::sync::Arc::new(TauriEvents(app)),
        request,
    )
}

#[tauri::command]
pub fn job_cancel(state: State<'_, AppState>, id: String) -> bool {
    state.jobs.cancel(&id)
}

/// Answer a confirmation a job asked for (`confirm` event). False when it no longer waits.
#[tauri::command]
pub fn confirm_answer(
    state: State<'_, AppState>,
    id: String,
    answer: crate::safety::ConfirmAnswer,
) -> bool {
    state.jobs.confirms().answer(&id, answer)
}

/// Show a finished export's file in the file manager (runs a short-lived process).
#[tauri::command]
pub async fn job_reveal(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let jobs = state.jobs.clone();
    tauri::async_runtime::spawn_blocking(move || ops::export::reveal(&jobs, &id))
        .await
        .map_err(|e| CommandError::new("io", format!("Couldn't show the file: {e}")))?
}

// ---- operate (M2b): each `op_*` returns the job id at once; the rest arrives as events ----

use crate::ops::operate;
use crate::ops::operate_types::*;
use dispatched_core::namespace::SubQueue;
use dispatched_core::ops::PurgeScope;
use std::sync::Arc;

fn events(app: AppHandle) -> Arc<dyn crate::events::Events> {
    Arc::new(TauriEvents(app))
}

#[tauri::command]
pub async fn op_send(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    form: SendForm,
) -> Result<JobStarted, CommandError> {
    operate::send(&state, events(app), &profile_id, form).await
}

#[tauri::command]
pub async fn op_import(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    form: ImportForm,
) -> Result<JobStarted, CommandError> {
    operate::import(&state, events(app), &profile_id, form).await
}

#[tauri::command]
pub async fn op_cancel_scheduled(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    entity: String,
    sequences: Vec<Seq>,
) -> Result<JobStarted, CommandError> {
    operate::cancel_scheduled(&state, events(app), &profile_id, &entity, sequences).await
}

#[tauri::command]
pub async fn op_receive(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    form: ReceiveForm,
) -> Result<JobStarted, CommandError> {
    operate::receive(&state, events(app), &profile_id, form).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn op_settle_deferred(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    entity: String,
    sub_queue: SubQueue,
    sequences: Vec<Seq>,
    session: Option<String>,
    how: SettlementForm,
) -> Result<JobStarted, CommandError> {
    operate::settle_deferred(
        &state,
        events(app),
        &profile_id,
        &entity,
        sub_queue,
        sequences,
        session,
        how,
    )
    .await
}

#[tauri::command]
pub async fn op_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    form: SelectForm,
) -> Result<JobStarted, CommandError> {
    operate::delete(&state, events(app), &profile_id, form).await
}

#[tauri::command]
pub async fn op_move(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    form: SelectForm,
    target: String,
    drop_uncopyable: bool,
) -> Result<JobStarted, CommandError> {
    operate::move_messages(
        &state,
        events(app),
        &profile_id,
        form,
        target,
        drop_uncopyable,
    )
    .await
}

#[tauri::command]
pub async fn op_resubmit(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    form: ResubmitForm,
) -> Result<JobStarted, CommandError> {
    operate::resubmit(&state, events(app), &profile_id, form).await
}

#[tauri::command]
pub async fn purge_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    entities: Vec<String>,
    scope: PurgeScope,
) -> Result<PurgePlans, CommandError> {
    operate::purge_plan(&state, events(app), &profile_id, entities, scope).await
}

#[tauri::command]
pub async fn op_purge(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    plan_id: String,
) -> Result<JobStarted, CommandError> {
    operate::purge(&state, events(app), &profile_id, &plan_id).await
}

#[tauri::command]
pub async fn dlq_summary(
    state: State<'_, AppState>,
    profile_id: String,
    entity: String,
    sub_queue: SubQueue,
) -> Result<DlqSummaryView, CommandError> {
    operate::dlq_summary(&state, &profile_id, &entity, sub_queue).await
}

#[tauri::command]
pub async fn session_state_get(
    state: State<'_, AppState>,
    profile_id: String,
    entity: String,
    session: String,
) -> Result<Option<SessionStateView>, CommandError> {
    operate::session_state_get(&state, &profile_id, &entity, &session).await
}

#[tauri::command]
pub async fn op_session_state_set(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    entity: String,
    session: String,
    value: Option<String>,
    allow_large: bool,
) -> Result<JobStarted, CommandError> {
    operate::session_state_set(
        &state,
        events(app),
        &profile_id,
        &entity,
        &session,
        value,
        allow_large,
    )
    .await
}

#[tauri::command]
pub async fn resolve_kind(
    state: State<'_, AppState>,
    profile_id: String,
    entity: String,
) -> Result<Option<dispatched_core::EntityKind>, CommandError> {
    operate::resolve_kind(&state, &profile_id, &entity).await
}

/// A picked file, by name, for the sheet to show.
#[derive(Serialize)]
pub struct OpenedFiles {
    pub pick: String,
    pub names: Vec<String>,
}

/// Open the native file dialog (Rust-side): `body` (several files, sent round-robin),
/// `template` (one incumbent BrokeredMessageTemplate, JSON or XML) or `import` (one export file).
/// The webview gets a pick id, never a path.
#[tauri::command]
pub async fn pick_open_paths(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    purpose: String,
) -> Result<Option<OpenedFiles>, CommandError> {
    use tauri_plugin_dialog::DialogExt;
    let multiple = purpose == "body";
    let filters: (&str, &[&str]) = match purpose.as_str() {
        "template" => ("Message template", &["json", "xml"]),
        "import" => ("Exported messages", &["json", "ndjson"]),
        "body" => ("Any file", &["*"]),
        _ => return Err(CommandError::new("invalid", "Unknown file purpose.")),
    };
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let d = app.dialog().file().set_parent(&window);
        let d = if filters.1 == ["*"] {
            d
        } else {
            d.add_filter(filters.0, filters.1)
        };
        if multiple {
            d.blocking_pick_files().unwrap_or_default()
        } else {
            d.blocking_pick_file().into_iter().collect()
        }
    })
    .await
    .map_err(|e| CommandError::new("io", format!("The file dialog failed: {e}")))?;
    let paths: Vec<std::path::PathBuf> = picked
        .into_iter()
        .filter_map(|p| p.into_path().ok())
        .collect();
    if paths.is_empty() {
        return Ok(None);
    }
    let names = paths
        .iter()
        .map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .collect();
    Ok(Some(OpenedFiles {
        pick: state.jobs.open_pick(paths),
        names,
    }))
}

#[tauri::command]
pub fn job_report(
    state: State<'_, AppState>,
    id: String,
    offset: u64,
    limit: u64,
    only_problems: bool,
) -> Option<ReportPage> {
    operate::job_report(&state.jobs, &id, offset, limit, only_problems)
}

/// The save dialog for a job's report.
#[tauri::command]
pub async fn pick_report_path(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    id: String,
    format: String,
) -> Result<Option<PickedPath>, CommandError> {
    use tauri_plugin_dialog::DialogExt;
    let stored = state
        .jobs
        .report(&id)
        .ok_or_else(|| CommandError::new("not_found", "That job has no report."))?;
    let ext = if format.eq_ignore_ascii_case("ndjson") {
        "ndjson"
    } else {
        "json"
    };
    let name = format!(
        "{}-{}-report.{ext}",
        stored.report.entity.replace(['/', '$'], "-"),
        kind_slug(&stored.report)
    );
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_parent(&window)
            .set_file_name(&name)
            .add_filter(ext.to_uppercase(), &[ext])
            .blocking_save_file()
    })
    .await
    .map_err(|e| CommandError::new("io", format!("The save dialog failed: {e}")))?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|path| ops::export::remember_pick(&state.jobs, path)))
}

fn kind_slug(r: &dispatched_core::jobs::JobReport) -> String {
    serde_json::to_value(r.kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "job".into())
}

#[tauri::command]
pub fn job_report_save(
    state: State<'_, AppState>,
    id: String,
    pick: String,
    format: String,
) -> Result<String, CommandError> {
    operate::save_report(&state.jobs, &id, &pick, &format)
}

// ---- the shared activity log ----

use crate::ops::activity::ActivityQueryForm;
use dispatched_core::activity::ActivityEntry;

#[tauri::command]
pub fn activity_list(
    state: State<'_, AppState>,
    query: ActivityQueryForm,
) -> Result<Vec<ActivityEntry>, CommandError> {
    ops::activity::list(ops::activity::log(&state)?, &query)
}

#[tauri::command]
pub fn activity_since(
    state: State<'_, AppState>,
    after_id: Option<i64>,
) -> Result<Vec<ActivityEntry>, CommandError> {
    ops::activity::since(ops::activity::log(&state)?, after_id)
}

/// The save dialog for an activity export.
#[tauri::command]
pub async fn pick_activity_export_path(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    format: String,
) -> Result<Option<PickedPath>, CommandError> {
    use tauri_plugin_dialog::DialogExt;
    let ext = if format.eq_ignore_ascii_case("ndjson") {
        "ndjson"
    } else {
        "json"
    };
    let name = format!("dispatched-activity.{ext}");
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_parent(&window)
            .set_file_name(&name)
            .add_filter(ext.to_uppercase(), &[ext])
            .blocking_save_file()
    })
    .await
    .map_err(|e| CommandError::new("io", format!("The save dialog failed: {e}")))?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|path| ops::export::remember_pick(&state.jobs, path)))
}

#[tauri::command]
pub fn activity_export(
    state: State<'_, AppState>,
    query: ActivityQueryForm,
    pick: String,
    format: String,
) -> Result<u64, CommandError> {
    ops::activity::export(
        ops::activity::log(&state)?,
        &state.jobs,
        &query,
        &pick,
        &format,
    )
}

/// Save a session's current state to a file the save dialog picks (Rust-side), read fresh, so a
/// state about to be replaced or cleared can be kept. The file name of what was written, or
/// `None` when the dialog was cancelled or the session has no state.
#[tauri::command]
pub async fn save_session_state(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    profile_id: String,
    entity: String,
    session: String,
) -> Result<Option<String>, CommandError> {
    use tauri_plugin_dialog::DialogExt;
    let active = state.active_for(&profile_id).await?;
    let Some(bytes) = active
        .explorer
        .session_state(&dispatched_core::EntityPath::parse(&entity)?, &session)
        .await?
    else {
        return Ok(None);
    };
    let name = format!("{}-state.bin", session.replace(['/', '\\', ':'], "-"));
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_parent(&window)
            .set_file_name(&name)
            .blocking_save_file()
    })
    .await
    .map_err(|e| CommandError::new("io", format!("The save dialog failed: {e}")))?;
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    std::fs::write(&path, &bytes)
        .map_err(|e| CommandError::new("io", format!("Could not write {}: {e}", path.display())))?;
    Ok(path.file_name().map(|n| n.to_string_lossy().into_owned()))
}
