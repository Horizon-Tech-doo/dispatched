//! The Dashboard: counts of every queue and subscription.

use crate::error::CommandError;
use crate::events::{Events, tracked_failures};
use crate::state::AppState;
use dispatched_core::namespace::known_entities;
use dispatched_core::{EntityCounts, Error, QueueInfo, SubscriptionInfo};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Topics whose subscriptions are listed at the same time.
const PARALLEL_TOPICS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RowKind {
    Queue,
    Subscription,
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardRow {
    pub kind: RowKind,
    /// The entity's tab id: a queue name or `topic/subscriptions/name`.
    pub id: String,
    pub name: String,
    pub topic: Option<String>,
    pub status: Option<String>,
    pub counts: Option<EntityCounts>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardFailure {
    pub topic: String,
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardView {
    pub rows: Vec<DashboardRow>,
    pub failures: Vec<DashboardFailure>,
    /// Why the dashboard shows only known entities.
    pub limited: Option<String>,
}

pub fn rows_from(
    queues: Vec<QueueInfo>,
    subscriptions: Vec<(String, Result<Vec<SubscriptionInfo>, Error>)>,
) -> DashboardView {
    let mut rows: Vec<DashboardRow> = queues
        .into_iter()
        .map(|q| DashboardRow {
            kind: RowKind::Queue,
            id: q.name.clone(),
            name: q.name,
            topic: None,
            status: q.status,
            counts: q.counts,
        })
        .collect();
    let mut failures = Vec::new();
    for (topic, result) in subscriptions {
        match result {
            Ok(subs) => rows.extend(subs.into_iter().map(|s| DashboardRow {
                kind: RowKind::Subscription,
                id: format!("{}/subscriptions/{}", s.topic, s.name),
                name: s.name,
                topic: Some(s.topic),
                status: s.status,
                counts: s.counts,
            })),
            Err(e) => {
                let e = CommandError::from(e);
                failures.push(DashboardFailure {
                    topic,
                    kind: e.kind.to_string(),
                    message: e.message,
                });
            }
        }
    }
    DashboardView {
        rows,
        failures,
        limited: None,
    }
}

pub fn limited_view(
    entity_path: Option<&str>,
    pins: &[String],
    needed: Option<String>,
) -> DashboardView {
    let rows = known_entities(entity_path, pins)
        .into_iter()
        .map(|k| match k.subscription {
            Some(sub) => DashboardRow {
                kind: RowKind::Subscription,
                id: format!("{}/subscriptions/{sub}", k.entity),
                name: sub,
                topic: Some(k.entity),
                status: None,
                counts: None,
            },
            None => DashboardRow {
                kind: RowKind::Queue,
                id: k.entity.clone(),
                name: k.entity,
                topic: None,
                status: None,
                counts: None,
            },
        })
        .collect();
    let right = needed.unwrap_or_else(|| "Manage".into());
    DashboardView {
        rows,
        failures: vec![],
        limited: Some(format!(
            "This identity can't list the namespace (needs {right}). Showing pinned entities without counts."
        )),
    }
}

/// The Dashboard refreshes itself on a timer, so only failures are logged (first load included).
pub async fn dashboard(
    state: &AppState,
    ev: &dyn Events,
    profile_id: &str,
) -> Result<DashboardView, CommandError> {
    let active = state.active_for(profile_id).await?;
    let pins = state
        .store
        .get_by_id(&active.profile_id)?
        .map(|p| p.pinned_entities)
        .unwrap_or_default();
    tracked_failures(ev, "Loading the dashboard failed", async {
        let ex = active.explorer.clone();
        let (queues, topics) = match tokio::try_join!(ex.list_queues(), ex.list_topics()) {
            Ok(v) => v,
            Err(Error::Forbidden { needed, .. }) => {
                return Ok(limited_view(ex.namespace().entity_path(), &pins, needed));
            }
            Err(e) => return Err(e.into()),
        };
        let gate = Arc::new(Semaphore::new(PARALLEL_TOPICS));
        let mut set = tokio::task::JoinSet::new();
        for t in topics {
            let (ex, gate) = (ex.clone(), gate.clone());
            set.spawn(async move {
                let _permit = gate.acquire_owned().await.expect("semaphore open");
                let r = ex.list_subscriptions(&t.name).await;
                (t.name, r)
            });
        }
        let mut subs = Vec::new();
        while let Some(joined) = set.join_next().await {
            subs.push(joined.map_err(|e| {
                CommandError::new("protocol", format!("dashboard task failed: {e}"))
            })?);
        }
        subs.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(rows_from(queues, subs))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::{EntityCounts, QueueInfo, SubscriptionInfo};

    fn counts(active: u64, dlq: u64) -> Option<EntityCounts> {
        Some(EntityCounts {
            active,
            dead_letter: dlq,
            ..Default::default()
        })
    }
    fn q(name: &str) -> QueueInfo {
        QueueInfo {
            name: name.into(),
            status: Some("Active".into()),
            counts: counts(1, 0),
            size_in_bytes: None,
            requires_session: false,
            max_delivery_count: None,
            lock_duration: None,
            forward_to: None,
        }
    }
    fn s(topic: &str, name: &str) -> SubscriptionInfo {
        SubscriptionInfo {
            topic: topic.into(),
            name: name.into(),
            status: None,
            counts: counts(5, 2),
            requires_session: false,
            max_delivery_count: None,
            forward_to: None,
        }
    }

    #[test]
    fn rows_are_queues_then_subscriptions_with_tab_ids() {
        let v = rows_from(
            vec![q("orders")],
            vec![("events".into(), Ok(vec![s("events", "audit")]))],
        );
        assert_eq!(v.rows.len(), 2);
        assert_eq!(
            (v.rows[0].kind, v.rows[0].id.as_str()),
            (RowKind::Queue, "orders")
        );
        assert_eq!(
            (
                v.rows[1].kind,
                v.rows[1].id.as_str(),
                v.rows[1].topic.as_deref()
            ),
            (
                RowKind::Subscription,
                "events/subscriptions/audit",
                Some("events")
            )
        );
        assert!(v.failures.is_empty() && v.limited.is_none());
    }

    #[test]
    fn a_topic_that_fails_is_reported_and_the_rest_still_shows() {
        let v = rows_from(
            vec![q("orders")],
            vec![
                (
                    "a".into(),
                    Err(Error::Forbidden {
                        needed: Some("Manage".into()),
                        detail: "no".into(),
                    }),
                ),
                ("b".into(), Ok(vec![s("b", "x")])),
            ],
        );
        assert_eq!(v.rows.len(), 2);
        assert_eq!(v.failures.len(), 1);
        assert_eq!(
            (v.failures[0].topic.as_str(), v.failures[0].kind.as_str()),
            ("a", "forbidden")
        );
    }

    #[test]
    fn limited_dashboard_lists_known_entities_without_counts() {
        let v = limited_view(
            Some("orders"),
            &["events/subscriptions/audit".to_string()],
            Some("Manage,EntityRead".into()),
        );
        assert_eq!(v.rows.len(), 2);
        assert!(v.rows.iter().all(|r| r.counts.is_none()));
        assert!(v.limited.unwrap().contains("Manage,EntityRead"));
    }

    #[test]
    fn row_json_shape() {
        let v = rows_from(vec![q("orders")], vec![]);
        let j = serde_json::to_value(&v.rows[0]).unwrap();
        assert_eq!(j["kind"], "queue");
        assert_eq!(j["counts"]["active"], 1);
    }
}
