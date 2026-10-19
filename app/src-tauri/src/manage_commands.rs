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

// ---- Manage jobs: each returns the job id at once; confirmations and the report are events ----

use crate::ops::export::{JobStarted, PickedPath};
use crate::ops::manage::{self, EntityRefForm, ExportedEntities, ImportPreview, PolicyChange};
use dispatched_core::mgmt::EntityStatus;
use std::sync::Arc;

fn events(app: AppHandle) -> Arc<dyn crate::events::Events> {
    Arc::new(TauriEvents(app))
}

#[tauri::command]
pub async fn op_entity_create(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    kind: EntityKind,
    entity: String,
    settings: Value,
    rule: Option<RuleForm>,
) -> Result<JobStarted, CommandError> {
    manage::entity_create(
        &state,
        events(app),
        &profile_id,
        kind,
        entity,
        settings,
        rule,
    )
    .await
}

#[tauri::command]
pub async fn op_entity_update(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    kind: EntityKind,
    entity: String,
    changes: Value,
) -> Result<JobStarted, CommandError> {
    manage::entity_update(&state, events(app), &profile_id, kind, entity, changes).await
}

#[tauri::command]
pub async fn op_entity_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    entities: Vec<EntityRefForm>,
) -> Result<JobStarted, CommandError> {
    manage::entity_delete(&state, events(app), &profile_id, entities).await
}

#[tauri::command]
pub async fn op_entity_status(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    target: EntityRefForm,
    status: EntityStatus,
) -> Result<JobStarted, CommandError> {
    manage::entity_status(&state, events(app), &profile_id, target, status).await
}

/// Duplicate, or with `rename` rename, an entity.
#[tauri::command]
pub async fn op_entity_copy(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    target: EntityRefForm,
    new_name: String,
    rename: bool,
) -> Result<JobStarted, CommandError> {
    manage::entity_copy(&state, events(app), &profile_id, target, new_name, rename).await
}

/// Add, or with `update` replace, a rule of subscription `entity`.
#[tauri::command]
pub async fn op_rule_save(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    entity: String,
    rule: RuleForm,
    update: bool,
) -> Result<JobStarted, CommandError> {
    manage::rule_save(&state, events(app), &profile_id, entity, rule, update).await
}

#[tauri::command]
pub async fn op_rule_remove(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    entity: String,
    name: String,
) -> Result<JobStarted, CommandError> {
    manage::rule_remove(&state, events(app), &profile_id, entity, name).await
}

#[tauri::command]
pub async fn op_policy_change(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    target: EntityRefForm,
    name: String,
    change: PolicyChange,
) -> Result<JobStarted, CommandError> {
    manage::policy_change(&state, events(app), &profile_id, target, name, change).await
}

/// The save dialog for an entities export, named after the namespace and the local time.
#[tauri::command]
pub async fn pick_entities_export_path(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    namespace: String,
    local_offset_minutes: i32,
) -> Result<Option<PickedPath>, CommandError> {
    use tauri_plugin_dialog::DialogExt;
    let offset = time::UtcOffset::from_whole_seconds(local_offset_minutes.clamp(-1080, 1080) * 60)
        .unwrap_or(time::UtcOffset::UTC);
    let now = time::OffsetDateTime::now_utc().to_offset(offset);
    let stamp = format!(
        "{:04}{:02}{:02}-{:02}{:02}",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute()
    );
    let ns: String = namespace
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let name = format!("{ns}-entities-{stamp}.xml");
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_parent(&window)
            .set_file_name(&name)
            .add_filter("XML", &["xml"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| CommandError::new("io", format!("The save dialog failed: {e}")))?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|path| crate::ops::export::remember_pick(&state.jobs, path)))
}

#[tauri::command]
pub async fn entities_export(
    state: State<'_, AppState>,
    profile_id: String,
    entities: Vec<EntityRefForm>,
    include_keys: bool,
    pick: String,
) -> Result<ExportedEntities, CommandError> {
    manage::entities_export(&state, &profile_id, entities, include_keys, &pick).await
}

#[tauri::command]
pub async fn entities_import_preview(
    state: State<'_, AppState>,
    profile_id: String,
    pick: String,
) -> Result<ImportPreview, CommandError> {
    manage::entities_import_preview(&state, &profile_id, &pick).await
}

#[tauri::command]
pub async fn op_entities_import(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    preview_id: String,
    overwrite: bool,
) -> Result<JobStarted, CommandError> {
    manage::entities_import(&state, events(app), &profile_id, &preview_id, overwrite).await
}
