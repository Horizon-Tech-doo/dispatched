//! Manage reads: an entity's settings for the edit form, its SAS policies without keys, and
//! copying a policy's key or connection string (Rust writes the clipboard; nothing returns).

use super::manage_types::{SettingsView, policy_connection_string, settings_view};
use crate::error::CommandError;
use crate::events::{Events, tracked, tracked_failures};
use dispatched_core::EntityKind;
use dispatched_core::mgmt::{AccessPolicy, EntityRef, KeySlot, PolicyView, policies::views};
use secrecy::ExposeSecret;

/// What to copy of a policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicySecret {
    ConnectionString,
    Key,
}

fn no_policies() -> CommandError {
    CommandError::new("invalid", "Subscriptions have no shared access policies.")
}

/// A queue or topic that can hold policies.
fn policy_entity(kind: EntityKind, entity: &str) -> Result<EntityRef, CommandError> {
    if kind == EntityKind::Subscription {
        return Err(no_policies());
    }
    Ok(EntityRef::parse(kind, entity)?)
}

/// The entity's settings, read fresh. Only failures are logged (the form opens it).
pub async fn entity_settings(
    state: &crate::state::AppState,
    ev: &dyn Events,
    profile_id: &str,
    kind: EntityKind,
    entity: &str,
) -> Result<SettingsView, CommandError> {
    let target = EntityRef::parse(kind, entity)?;
    let active = state.active_for(profile_id).await?;
    tracked_failures(
        ev,
        &format!("Reading the settings of {entity} failed"),
        async {
            let m = active.explorer.management_client();
            Ok(match &target {
                EntityRef::Queue { name } => {
                    settings_view(kind, entity, &m.get_queue_description(name).await?.settings)
                }
                EntityRef::Topic { name } => {
                    settings_view(kind, entity, &m.get_topic_description(name).await?.settings)
                }
                EntityRef::Subscription { topic, name } => settings_view(
                    kind,
                    entity,
                    &m.get_subscription(topic, name).await?.settings,
                ),
            })
        },
    )
    .await
}

async fn read_policies(
    explorer: &dispatched_core::Explorer,
    target: &EntityRef,
) -> Result<Vec<AccessPolicy>, CommandError> {
    let m = explorer.management_client();
    Ok(match target {
        EntityRef::Queue { name } => m.get_queue_description(name).await?.policies,
        EntityRef::Topic { name } => m.get_topic_description(name).await?.policies,
        EntityRef::Subscription { .. } => return Err(no_policies()),
    })
}

/// The entity's SAS policies: names and rights, never keys.
pub async fn entity_policies(
    state: &crate::state::AppState,
    ev: &dyn Events,
    profile_id: &str,
    kind: EntityKind,
    entity: &str,
) -> Result<Vec<PolicyView>, CommandError> {
    let target = policy_entity(kind, entity)?;
    let active = state.active_for(profile_id).await?;
    tracked_failures(
        ev,
        &format!("Reading the policies of {entity} failed"),
        async { Ok(views(&read_policies(&active.explorer, &target).await?)) },
    )
    .await
}

/// Copy a policy's key or connection string with `put` (the clipboard). The secret never
/// returns to the webview; the read is not an activity entry, only its outcome line is.
#[allow(clippy::too_many_arguments)]
pub async fn copy_policy_secret(
    state: &crate::state::AppState,
    ev: &dyn Events,
    profile_id: &str,
    kind: EntityKind,
    entity: &str,
    policy: &str,
    slot: KeySlot,
    what: PolicySecret,
    put: impl FnOnce(String) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    let target = policy_entity(kind, entity)?;
    let active = state.active_for(profile_id).await?;
    let label = match what {
        PolicySecret::ConnectionString => "connection string",
        PolicySecret::Key => "key",
    };
    tracked(
        ev,
        &format!("Copying the {label} of policy {policy} failed"),
        async {
            let policies = read_policies(&active.explorer, &target).await?;
            let p = policies
                .iter()
                .find(|p| p.key_name == policy)
                .ok_or_else(|| {
                    CommandError::new("not_found", format!("{entity} has no policy {policy}."))
                })?;
            let key = match slot {
                KeySlot::Primary => &p.primary_key,
                KeySlot::Secondary => &p.secondary_key,
            };
            let text = match what {
                PolicySecret::Key => key.expose_secret().to_string(),
                PolicySecret::ConnectionString => {
                    let ns = active.explorer.namespace();
                    policy_connection_string(
                        ns.host(),
                        ns.is_emulator(),
                        &target.path(),
                        policy,
                        key.expose_secret(),
                    )
                    .expose_secret()
                    .to_string()
                }
            };
            put(text)
        },
        |_| format!("Copied the {} {label} of policy {policy}", slot.as_str()),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Recorder;
    use crate::ops::test_util::state;

    #[tokio::test]
    async fn subscription_has_no_policies() {
        let s = state();
        let ev = Recorder::default();
        let e = entity_policies(
            &s,
            &ev,
            "p",
            EntityKind::Subscription,
            "events/subscriptions/all",
        )
        .await
        .unwrap_err();
        assert_eq!(e.kind, "invalid");
        let e = copy_policy_secret(
            &s,
            &ev,
            "p",
            EntityKind::Subscription,
            "events/subscriptions/all",
            "app",
            KeySlot::Primary,
            PolicySecret::ConnectionString,
            |_| panic!("nothing is copied"),
        )
        .await
        .unwrap_err();
        assert_eq!(e.kind, "invalid");
    }

    #[tokio::test]
    async fn reads_need_the_connected_profile() {
        let s = state();
        let ev = Recorder::default();
        let e = entity_settings(&s, &ev, "p", EntityKind::Queue, "orders")
            .await
            .unwrap_err();
        assert_eq!(e.kind, "not_connected");
        let e = entity_policies(&s, &ev, "p", EntityKind::Queue, "orders")
            .await
            .unwrap_err();
        assert_eq!(e.kind, "not_connected");
    }
}
