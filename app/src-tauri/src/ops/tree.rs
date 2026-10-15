use crate::error::CommandError;
use crate::events::{Events, tracked};
use crate::state::AppState;
use dispatched_core::namespace::known_entities;
use dispatched_core::{EntityPath, Error, QueueInfo, SubscriptionInfo, TopicInfo};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct EntityListing {
    pub queues: Vec<QueueInfo>,
    pub topics: Vec<TopicInfo>,
    /// Why the listing is limited (the identity can't list the namespace).
    pub limited: Option<String>,
    /// Entities we can still show: the connection string's EntityPath, then pins.
    pub known: Vec<EntityPath>,
}

/// Decide what the tree shows from a listing attempt.
pub fn listing_from(
    result: dispatched_core::Result<(Vec<QueueInfo>, Vec<TopicInfo>)>,
    entity_path: Option<&str>,
    pins: &[String],
) -> Result<EntityListing, CommandError> {
    match result {
        Ok((queues, topics)) => Ok(EntityListing {
            queues,
            topics,
            limited: None,
            known: vec![],
        }),
        Err(Error::Forbidden { needed, .. }) => {
            let known = known_entities(entity_path, pins);
            let right = needed.unwrap_or_else(|| "Manage".into());
            let limited = if known.is_empty() {
                format!(
                    "This identity can't list the namespace (needs {right}). Edit the profile to pin the entities you use."
                )
            } else {
                format!(
                    "This identity can't list the namespace (needs {right}). Showing pinned entities."
                )
            };
            Ok(EntityListing {
                queues: vec![],
                topics: vec![],
                limited: Some(limited),
                known,
            })
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn list_entities(
    state: &AppState,
    ev: &dyn Events,
) -> Result<EntityListing, CommandError> {
    let active = state.active().await?;
    let pins = state
        .store
        .get_by_id(&active.profile_id)?
        .map(|p| p.pinned_entities)
        .unwrap_or_default();
    tracked(
        ev,
        "Listing the namespace failed",
        async {
            let ex = &active.explorer;
            let result = tokio::try_join!(ex.list_queues(), ex.list_topics());
            listing_from(result, ex.namespace().entity_path(), &pins)
        },
        |l| match &l.limited {
            None => format!(
                "Listed {} queues and {} topics",
                l.queues.len(),
                l.topics.len()
            ),
            Some(_) => format!(
                "Listing not allowed; showing {} pinned entities",
                l.known.len()
            ),
        },
    )
    .await
}

pub async fn list_subscriptions(
    state: &AppState,
    ev: &dyn Events,
    topic: &str,
) -> Result<Vec<SubscriptionInfo>, CommandError> {
    let active = state.active().await?;
    tracked(
        ev,
        &format!("Listing subscriptions of {topic} failed"),
        async { Ok(active.explorer.list_subscriptions(topic).await?) },
        |subs| format!("Listed {} subscriptions of {topic}", subs.len()),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::{Error, QueueInfo};

    fn queue(name: &str) -> QueueInfo {
        QueueInfo {
            name: name.into(),
            status: None,
            counts: None,
            size_in_bytes: None,
            requires_session: false,
            max_delivery_count: None,
            lock_duration: None,
            forward_to: None,
        }
    }

    #[test]
    fn a_full_listing_passes_through() {
        let l = listing_from(Ok((vec![queue("orders")], vec![])), None, &[]).unwrap();
        assert_eq!(l.queues.len(), 1);
        assert!(l.limited.is_none() && l.known.is_empty());
    }

    #[test]
    fn list_entities_falls_back_to_known_entities_on_forbidden() {
        let forbidden = Err(Error::Forbidden {
            needed: Some("Manage,EntityRead".into()),
            detail: "no".into(),
        });
        let pins = vec!["events/subscriptions/audit".to_string()];
        let l = listing_from(forbidden, Some("orders"), &pins).unwrap();
        assert!(l.queues.is_empty() && l.topics.is_empty());
        assert_eq!(l.known.len(), 2);
        let why = l.limited.unwrap();
        assert!(
            why.contains("can't list") && why.contains("Manage"),
            "{why}"
        );
    }

    #[test]
    fn forbidden_with_nothing_known_still_explains_how_to_pin() {
        let forbidden = Err(Error::Forbidden {
            needed: None,
            detail: "no".into(),
        });
        let l = listing_from(forbidden, None, &[]).unwrap();
        assert!(l.known.is_empty());
        assert!(l.limited.unwrap().contains("pin"));
    }

    #[test]
    fn other_errors_are_errors() {
        let e = listing_from(Err(Error::Network("reset".into())), None, &[]).unwrap_err();
        assert_eq!(e.kind, "network");
    }
}
