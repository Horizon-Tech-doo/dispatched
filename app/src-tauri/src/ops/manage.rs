//! Manage jobs: entity, rule and policy changes, behind the core gate like every operate job
//! (`super::operate::start_op`), and the incumbent XML export (a read) and import (a job).

use super::manage_types::{RuleForm, settings_change};
use super::operate::{Done, report, start_op};
use crate::error::CommandError;
use crate::events::Events;
use crate::jobs::Jobs;
use crate::ops::export::JobStarted;
use crate::state::AppState;
use dispatched_core::EntityKind;
use dispatched_core::jobs::JobKind;
use dispatched_core::mgmt::incumbent::{ImportEntity, export_entities, parse_entities};
use dispatched_core::mgmt::{AccessRight, Description, EntityRef, EntityStatus, KeySlot, RuleSpec};
use dispatched_core::{EntitySpec, ExportFormat};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// An entity as the webview names it: its kind and its tab id (`topic/subscriptions/name`).
#[derive(Debug, Clone, Deserialize)]
pub struct EntityRefForm {
    pub kind: EntityKind,
    pub entity: String,
}

impl TryFrom<&EntityRefForm> for EntityRef {
    type Error = CommandError;

    fn try_from(f: &EntityRefForm) -> Result<Self, CommandError> {
        Ok(EntityRef::parse(f.kind, &f.entity)?)
    }
}

/// How an entity reads in a title: "queue orders", "subscription events/Subscriptions/all".
fn named(e: &EntityRef) -> String {
    format!("{} {}", e.kind().as_str(), e.path())
}

/// A job that changes one or more entities.
async fn run(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    kind: JobKind,
    entity: &EntityRef,
    title: String,
    work: impl FnOnce(
        dispatched_core::ops::Operator,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = dispatched_core::Result<dispatched_core::jobs::JobReport>,
                > + Send,
        >,
    > + Send
    + 'static,
) -> Result<JobStarted, CommandError> {
    start_op(
        state,
        ev,
        profile_id,
        kind,
        entity.path(),
        title,
        move |op, _b, _c| async move { report(work(op).await) },
    )
    .await
}

pub async fn entity_create(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    kind: EntityKind,
    entity: String,
    settings: Value,
    rule: Option<RuleForm>,
) -> Result<JobStarted, CommandError> {
    let target = EntityRef::parse(kind, &entity)?;
    let change = settings_change(kind, &settings, true)?;
    let spec = match &target {
        EntityRef::Queue { name } => {
            let mut d = Description::default();
            change.apply_queue(&mut d.settings, true)?;
            EntitySpec::Queue(name.clone(), d)
        }
        EntityRef::Topic { name } => {
            let mut d = Description::default();
            change.apply_topic(&mut d.settings, true)?;
            EntitySpec::Topic(name.clone(), d)
        }
        EntityRef::Subscription { topic, name } => {
            let mut d = Description::default();
            change.apply_subscription(&mut d.settings, true)?;
            EntitySpec::Subscription {
                topic: topic.clone(),
                name: name.clone(),
                d,
                rule: rule.map(RuleSpec::try_from).transpose()?,
            }
        }
    };
    let title = format!("Create {}", named(&target));
    run(
        state,
        ev,
        profile_id,
        JobKind::CreateEntities,
        &target,
        title,
        move |op| Box::pin(async move { op.create_entity(spec).await }),
    )
    .await
}

/// Only the given settings change. The core reads the description after the confirmation,
/// so what someone else changed meanwhile in other settings is kept.
pub async fn entity_update(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    kind: EntityKind,
    entity: String,
    settings: Value,
) -> Result<JobStarted, CommandError> {
    let target = EntityRef::parse(kind, &entity)?;
    let change = settings_change(kind, &settings, false)?;
    if change.is_empty() {
        return Err(CommandError::new("invalid", "Nothing to change."));
    }
    let title = format!("Update {}", named(&target));
    let t = target.clone();
    run(
        state,
        ev,
        profile_id,
        JobKind::UpdateEntities,
        &target,
        title,
        move |op| Box::pin(async move { op.update_settings(t, change).await }),
    )
    .await
}

/// "Delete queue orders", or "Delete 3 entities".
pub fn delete_title(targets: &[EntityRef]) -> String {
    match targets {
        [one] => format!("Delete {}", named(one)),
        many => format!("Delete {} entities", many.len()),
    }
}

pub async fn entity_delete(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    entities: Vec<EntityRefForm>,
) -> Result<JobStarted, CommandError> {
    let targets = entities
        .iter()
        .map(EntityRef::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let Some(first) = targets.first().cloned() else {
        return Err(CommandError::new("invalid", "Choose what to delete."));
    };
    let title = delete_title(&targets);
    let entity = if targets.len() == 1 {
        first.path()
    } else {
        format!("{} entities", targets.len())
    };
    start_op(
        state,
        ev,
        profile_id,
        JobKind::DeleteEntities,
        entity,
        title,
        move |op, _b, _c| async move { report(op.delete_entities(targets).await) },
    )
    .await
}

pub async fn entity_status(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    target: EntityRefForm,
    status: EntityStatus,
) -> Result<JobStarted, CommandError> {
    let target = EntityRef::try_from(&target)?;
    let title = format!("Set {} to {status}", named(&target));
    let t = target.clone();
    run(
        state,
        ev,
        profile_id,
        JobKind::UpdateEntities,
        &target,
        title,
        move |op| Box::pin(async move { op.set_status(t, status).await }),
    )
    .await
}

/// Duplicate (`rename == false`) or rename an entity.
pub async fn entity_copy(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    target: EntityRefForm,
    new_name: String,
    rename: bool,
) -> Result<JobStarted, CommandError> {
    let target = EntityRef::try_from(&target)?;
    let new_name = new_name.trim().to_string();
    if new_name.is_empty() || new_name.contains('/') {
        return Err(CommandError::new(
            "invalid",
            "The new name must be a plain name, without '/'.",
        ));
    }
    let (kind, verb) = if rename {
        (JobKind::Rename, "Rename")
    } else {
        (JobKind::Duplicate, "Duplicate")
    };
    let title = format!("{verb} {} as {new_name}", named(&target));
    let t = target.clone();
    run(state, ev, profile_id, kind, &target, title, move |op| {
        Box::pin(async move {
            if rename {
                op.rename(t, new_name).await
            } else {
                op.duplicate(t, new_name).await
            }
        })
    })
    .await
}

fn subscription(entity: &str) -> Result<EntityRef, CommandError> {
    Ok(EntityRef::parse(EntityKind::Subscription, entity)?)
}

/// Add (`update == false`) or update a rule.
pub async fn rule_save(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    entity: String,
    rule: RuleForm,
    update: bool,
) -> Result<JobStarted, CommandError> {
    let sub = subscription(&entity)?;
    let rule = RuleSpec::try_from(rule)?;
    let verb = if update { "Update" } else { "Add" };
    let title = format!("{verb} rule {} of {}", rule.name, sub.path());
    let s = sub.clone();
    run(
        state,
        ev,
        profile_id,
        JobKind::UpdateEntities,
        &sub,
        title,
        move |op| {
            Box::pin(async move {
                if update {
                    op.update_rule(s, rule).await
                } else {
                    op.add_rule(s, rule).await
                }
            })
        },
    )
    .await
}

pub async fn rule_remove(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    entity: String,
    name: String,
) -> Result<JobStarted, CommandError> {
    let sub = subscription(&entity)?;
    let title = format!("Remove rule {name} from {}", sub.path());
    let s = sub.clone();
    run(
        state,
        ev,
        profile_id,
        JobKind::UpdateEntities,
        &sub,
        title,
        move |op| Box::pin(async move { op.remove_rule(s, name).await }),
    )
    .await
}

/// A policy change: add with rights, remove, or regenerate one key.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PolicyChange {
    Add { rights: Vec<AccessRight> },
    Remove,
    Renew { slot: KeySlot },
}

pub async fn policy_change(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    target: EntityRefForm,
    name: String,
    change: PolicyChange,
) -> Result<JobStarted, CommandError> {
    let target = EntityRef::try_from(&target)?;
    let title = match &change {
        PolicyChange::Add { .. } => format!("Add policy {name} to {}", target.path()),
        PolicyChange::Remove => format!("Remove policy {name} from {}", target.path()),
        PolicyChange::Renew { slot } => {
            format!(
                "Regenerate the {} key of policy {name} on {}",
                slot.as_str(),
                target.path()
            )
        }
    };
    let t = target.clone();
    run(
        state,
        ev,
        profile_id,
        JobKind::UpdateEntities,
        &target,
        title,
        move |op| {
            Box::pin(async move {
                match change {
                    PolicyChange::Add { rights } => op.add_policy(t, name, rights).await,
                    PolicyChange::Remove => op.remove_policy(t, name).await,
                    PolicyChange::Renew { slot } => op.regenerate_key(t, name, slot).await,
                }
            })
        },
    )
    .await
}

/// `serviceBusNamespace` of an export: the first label of the host.
pub fn namespace_name(host: &str) -> &str {
    host.split('.').next().unwrap_or(host)
}

/// A finished export.
#[derive(Debug, Clone, Serialize)]
pub struct ExportedEntities {
    pub count: usize,
    pub file_name: String,
}

/// Export `entities` (all queues and topics when empty) to the incumbent's XML, written to
/// the path a save dialog picked. A read: not gated, not logged as a change. With keys the
/// file is owner-only.
pub async fn entities_export(
    state: &AppState,
    profile_id: &str,
    entities: Vec<EntityRefForm>,
    include_keys: bool,
    pick: &str,
) -> Result<ExportedEntities, CommandError> {
    let refs = entities
        .iter()
        .map(EntityRef::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let active = state.active_for(profile_id).await?;
    let path = state
        .jobs
        .take_pick(pick)
        .ok_or_else(|| CommandError::new("invalid", "Choose where to save the export."))?;
    let collected = active.explorer.collect_for_export(&refs).await;
    let collected = match collected {
        Ok(c) => c,
        Err(e) => {
            state.jobs.restore_pick(pick, path);
            return Err(e.into());
        }
    };
    let ns = namespace_name(active.explorer.namespace().host()).to_string();
    let xml = export_entities(&ns, &collected, include_keys)?;
    let count = collected
        .iter()
        .map(|e| match e {
            dispatched_core::mgmt::incumbent::ExportEntity::Queue { .. } => 1,
            dispatched_core::mgmt::incumbent::ExportEntity::Topic { subscriptions, .. } => {
                1 + subscriptions.len()
            }
        })
        .sum();
    // The save dialog asked "replace?" already.
    let mut file = if include_keys {
        dispatched_core::FileExport::create_private(&path, ExportFormat::Json, true)?
    } else {
        dispatched_core::FileExport::create(&path, ExportFormat::Json, true)?
    };
    file.write_raw(xml.as_bytes())?;
    file.finish()?;
    Ok(ExportedEntities {
        count,
        file_name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    })
}

/// One entity of an import file, as the preview lists it.
#[derive(Debug, Clone, Serialize)]
pub struct PreviewRow {
    pub kind: EntityKind,
    pub entity: String,
    pub rules: usize,
    pub policies: usize,
    /// Some policy of it carries keys in the file.
    pub keys: bool,
}

/// An import file read and waiting for its confirmation.
#[derive(Debug, Clone, Serialize)]
pub struct ImportPreview {
    pub preview_id: String,
    pub entities: Vec<PreviewRow>,
}

pub fn preview_rows(items: &[ImportEntity]) -> Vec<PreviewRow> {
    items
        .iter()
        .map(|it| {
            let e = it.entity();
            let (rules, policies, keys) = match it {
                ImportEntity::Queue { policies, .. } | ImportEntity::Topic { policies, .. } => {
                    (0, policies.len(), policies.iter().any(|p| p.has_keys()))
                }
                ImportEntity::Subscription { rules, .. } => (rules.len(), 0, false),
            };
            PreviewRow {
                kind: e.kind(),
                entity: e.path(),
                rules,
                policies,
                keys,
            }
        })
        .collect()
}

/// Larger import files are refused: an export of a namespace is a few megabytes at most.
const IMPORT_LIMIT: u64 = 64 * 1024 * 1024;

/// Read and check the incumbent file the open dialog returned; nothing changes yet.
pub async fn entities_import_preview(
    state: &AppState,
    profile_id: &str,
    pick: &str,
) -> Result<ImportPreview, CommandError> {
    state.active_for(profile_id).await?;
    let path = state
        .jobs
        .take_open(pick)
        .and_then(|p| p.into_iter().next())
        .ok_or_else(|| CommandError::new("invalid", "Choose the file again."))?;
    let size = std::fs::metadata(&path)
        .map_err(|e| CommandError::new("io", format!("Could not read the file: {e}")))?
        .len();
    if size > IMPORT_LIMIT {
        return Err(CommandError::new(
            "invalid",
            "The file is larger than 64 MiB; that is not an entities export.",
        ));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| CommandError::new("io", format!("Could not read the file: {e}")))?;
    let items = parse_entities(&text)?;
    if items.is_empty() {
        return Err(CommandError::new(
            "invalid",
            "The file holds no queue, topic or subscription.",
        ));
    }
    let entities = preview_rows(&items);
    Ok(ImportPreview {
        preview_id: state.jobs.preview(profile_id, items),
        entities,
    })
}

/// The entities of a preview, once, on the profile it was read for.
pub fn take_preview(
    jobs: &Jobs,
    profile_id: &str,
    preview_id: &str,
) -> Result<Vec<ImportEntity>, CommandError> {
    jobs.take_preview(preview_id)
        .filter(|p| p.profile_id == profile_id)
        .map(|p| p.items)
        .ok_or_else(|| {
            CommandError::new(
                "not_found",
                "Choose the file again: it was read for another import.",
            )
        })
}

pub async fn entities_import(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    preview_id: &str,
    overwrite: bool,
) -> Result<JobStarted, CommandError> {
    let items = take_preview(&state.jobs, profile_id, preview_id)?;
    let title = format!(
        "Import {} {}",
        items.len(),
        if items.len() == 1 {
            "entity"
        } else {
            "entities"
        }
    );
    let entity = format!("{} entities", items.len());
    start_op(
        state,
        ev,
        profile_id,
        JobKind::ImportEntities,
        entity,
        title,
        move |op, b, c| async move {
            op.import_entities(items, overwrite, &*b, &c)
                .await
                .map(|r| Done::Report(r, Vec::new()))
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Recorder;
    use crate::ops::test_util::state;
    use dispatched_core::mgmt::incumbent::parse_entities;
    use std::sync::Arc;

    const FILE: &str =
        include_str!("../../../../crates/dispatched-core/tests/fixtures/incumbent_entities.xml");

    #[tokio::test]
    async fn empty_update_is_refused_before_a_job() {
        let s = state();
        let ev = Arc::new(Recorder::default());
        let e = entity_update(
            &s,
            ev,
            "p",
            EntityKind::Queue,
            "orders".into(),
            serde_json::json!({}),
        )
        .await
        .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            ("invalid", "Nothing to change.")
        );
        assert!(!s.jobs.any_running(), "no job was registered");
    }

    #[test]
    fn preview_runs_once_on_its_profile() {
        let s = state();
        let items = parse_entities(FILE).unwrap();
        let id = s.jobs.preview("p", items.clone());
        assert!(take_preview(&s.jobs, "q", &id).is_err(), "another profile");
        let id = s.jobs.preview("p", items);
        assert!(take_preview(&s.jobs, "p", &id).is_ok());
        let e = take_preview(&s.jobs, "p", &id).unwrap_err();
        assert_eq!(e.kind, "not_found");
    }

    #[test]
    fn import_preview_lists_rules_and_policies() {
        let items = parse_entities(FILE).unwrap();
        let rows = preview_rows(&items);
        assert_eq!(rows.len(), items.len());
        assert!(
            rows.iter()
                .any(|r| r.kind == EntityKind::Subscription && r.rules > 0)
        );
        assert!(
            rows.iter()
                .all(|r| r.kind != EntityKind::Subscription || r.policies == 0)
        );
    }

    #[test]
    fn delete_title_names_one_or_counts_many() {
        assert_eq!(
            delete_title(&[EntityRef::queue("orders")]),
            "Delete queue orders"
        );
        assert_eq!(
            delete_title(&[
                EntityRef::queue("a"),
                EntityRef::topic("b"),
                EntityRef::subscription("b", "c")
            ]),
            "Delete 3 entities"
        );
    }

    #[test]
    fn namespace_name_is_the_first_label() {
        assert_eq!(namespace_name("myns.servicebus.windows.net"), "myns");
        assert_eq!(namespace_name("localhost"), "localhost");
    }
}
