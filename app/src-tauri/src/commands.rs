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

use crate::ops::account::{AccountView, SignInView};
use crate::ops::connection::ConnectionView;
use crate::ops::discovery::{DiscoveryView, TenantView};
use crate::ops::tree::EntityListing;
use dispatched_core::{
    EntityPath, PeekOptions, PeekView, PeekedMessage, SubQueue, SubscriptionInfo,
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

/// Kept from M0 for M1c's message views.
#[tauri::command]
pub async fn peek(
    state: State<'_, AppState>,
    entity: String,
    sub_queue: SubQueue,
    from_sequence: i64,
    max: u32,
) -> Result<Vec<PeekedMessage>, CommandError> {
    let path = EntityPath::parse(&entity)?;
    let view = match sub_queue {
        SubQueue::Active => PeekView::Active,
        SubQueue::DeadLetter => PeekView::DeadLetter,
        SubQueue::TransferDeadLetter => PeekView::TransferDeadLetter,
    };
    let opts = PeekOptions::new(max.min(500)).from_sequence(from_sequence);
    let page = state
        .active()
        .await?
        .explorer
        .peek(&path, view, &opts)
        .await?;
    Ok(page.messages)
}
