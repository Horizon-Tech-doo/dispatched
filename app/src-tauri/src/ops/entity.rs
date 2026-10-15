//! One entity: its details, a subscription's rules, a session-enabled entity's sessions.

use crate::error::CommandError;
use crate::events::{Events, tracked, tracked_failures};
use crate::ops::tree::count;
use crate::state::AppState;
use dispatched_core::{EntityDetails, EntityKind, EntityPath, RuleInfo, SessionListing};

/// Overview polls this every 10 s while it is visible, so only failures are logged.
pub async fn entity_details(
    state: &AppState,
    ev: &dyn Events,
    profile_id: &str,
    kind: EntityKind,
    entity: &str,
) -> Result<EntityDetails, CommandError> {
    let active = state.active_for(profile_id).await?;
    tracked_failures(
        ev,
        &format!("Reading the details of {entity} failed"),
        async { Ok(active.explorer.entity_details(kind, entity).await?) },
    )
    .await
}

pub async fn list_rules(
    state: &AppState,
    ev: &dyn Events,
    profile_id: &str,
    entity: &str,
) -> Result<Vec<RuleInfo>, CommandError> {
    let path = EntityPath::parse(entity)?;
    let Some(sub) = path.subscription.clone() else {
        return Err(CommandError::new(
            "invalid",
            format!("{entity} is not a subscription; only subscriptions have rules."),
        ));
    };
    let active = state.active_for(profile_id).await?;
    tracked(
        ev,
        &format!("Listing the rules of {path} failed"),
        async { Ok(active.explorer.list_rules(&path.entity, &sub).await?) },
        |r| format!("Listed {} of {path}", count(r.len(), "rule", "rules")),
    )
    .await
}

pub async fn list_sessions(
    state: &AppState,
    ev: &dyn Events,
    profile_id: &str,
    entity: &str,
) -> Result<SessionListing, CommandError> {
    let path = EntityPath::parse(entity)?;
    let active = state.active_for(profile_id).await?;
    tracked(
        ev,
        &format!("Listing the sessions of {path} failed"),
        async { Ok(active.explorer.list_sessions(&path).await?) },
        |l| {
            format!(
                "Listed {} of {path}",
                count(l.sessions.len(), "session", "sessions")
            )
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Recorder;
    use crate::ops::test_util::state;

    #[tokio::test]
    async fn everything_needs_the_connected_profile_and_valid_paths() {
        let s = state();
        let ev = Recorder::default();
        assert_eq!(
            entity_details(&s, &ev, "p", EntityKind::Queue, "orders")
                .await
                .unwrap_err()
                .kind,
            "not_connected"
        );
        assert_eq!(
            list_sessions(&s, &ev, "p", "orders")
                .await
                .unwrap_err()
                .kind,
            "not_connected"
        );
        let e = list_rules(&s, &ev, "p", "orders").await.unwrap_err();
        assert_eq!(e.kind, "invalid", "a queue has no rules: {}", e.message);
    }
}
