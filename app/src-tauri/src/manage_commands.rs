//! Tauri commands of the manage screens and the listener (M3b). Like `commands`, they only
//! unwrap Tauri's state and handle; the work is in `ops::manage*` and `ops::listen`.

use crate::error::CommandError;
use crate::events::TauriEvents;
use crate::ops::manage_read::{self, PolicySecret};
use crate::ops::manage_types::{self, FormIssue, RuleForm, SettingsView};
use crate::state::AppState;
use dispatched_core::EntityKind;
use dispatched_core::mgmt::{KeySlot, PolicyView};
use serde_json::Value;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn entity_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    kind: EntityKind,
    entity: String,
) -> Result<SettingsView, CommandError> {
    manage_read::entity_settings(&state, &TauriEvents(app), &profile_id, kind, &entity).await
}

#[tauri::command]
pub async fn entity_policies(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    kind: EntityKind,
    entity: String,
) -> Result<Vec<PolicyView>, CommandError> {
    manage_read::entity_policies(&state, &TauriEvents(app), &profile_id, kind, &entity).await
}

/// Pure and local.
#[tauri::command]
pub fn check_settings(kind: EntityKind, form: Value, create: bool) -> Vec<FormIssue> {
    manage_types::check_settings(kind, &form, create)
}

/// Pure and local.
#[tauri::command]
pub fn check_rule(rule: RuleForm) -> Vec<FormIssue> {
    manage_types::rule_issues(&rule)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn copy_policy_secret(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    kind: EntityKind,
    entity: String,
    policy: String,
    slot: KeySlot,
    what: PolicySecret,
) -> Result<(), CommandError> {
    manage_read::copy_policy_secret(
        &state,
        &TauriEvents(app),
        &profile_id,
        kind,
        &entity,
        &policy,
        slot,
        what,
        crate::clipboard::write_secret_text,
    )
    .await
}
