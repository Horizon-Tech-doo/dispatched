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
) -> Result<Option<PickedPath>, CommandError> {
    use tauri_plugin_dialog::DialogExt;
    // `time` can't read the local offset in a multi-threaded process on Unix, so the name is UTC.
    let name =
        ops::export::suggested_file_name(&entity, view, &format, time::OffsetDateTime::now_utc());
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

/// Show a finished export's file in the file manager (runs a short-lived process).
#[tauri::command]
pub async fn job_reveal(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let jobs = state.jobs.clone();
    tauri::async_runtime::spawn_blocking(move || ops::export::reveal(&jobs, &id))
        .await
        .map_err(|e| CommandError::new("io", format!("Couldn't show the file: {e}")))?
}
