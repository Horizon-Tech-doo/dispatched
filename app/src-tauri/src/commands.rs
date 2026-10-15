//! Tauri commands: unwrap state and handle, call `ops`.

use crate::error::CommandError;
use crate::events::TauriEvents;
use crate::ops;
use crate::state::AppState;
use dispatched_core::diagnostics::ConnectionReport;
use dispatched_core::profiles::{Profile, ProfileDraft};
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

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>, key: String) -> Result<Option<Value>, CommandError> {
    ops::settings::get(&state, &key)
}

#[tauri::command]
pub fn settings_set(state: State<'_, AppState>, key: String, value: Option<Value>) -> Result<(), CommandError> {
    ops::settings::set(&state, &key, value)
}

#[tauri::command]
pub fn profiles_list(state: State<'_, AppState>) -> Result<Vec<Profile>, CommandError> {
    ops::profiles::list(&state)
}

#[tauri::command]
pub async fn profile_save(app: AppHandle, state: State<'_, AppState>, draft: ProfileDraft) -> Result<Profile, CommandError> {
    ops::profiles::save(&state, &TauriEvents(app), draft).await
}

#[tauri::command]
pub async fn profile_delete(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    ops::profiles::delete(&state, &TauriEvents(app), &id).await
}

#[tauri::command]
pub async fn profile_test(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<ConnectionReport, CommandError> {
    ops::profiles::test(&state, &TauriEvents(app), &id).await
}
