//! `dispatched show`, `session ls`, `rule ls`: read-only inspection.

use crate::Format;
use crate::commands::browse::explorer_for;
use crate::context::Ctx;
use crate::output::{sanitize, table, thousands};
use crate::resolve::Selection;
use dispatched_core::messaging::sessions::{
    BrokerList, MAX_SESSIONS, SessionListing, SessionSummary,
};
use dispatched_core::mgmt::{EntityDetails, EntityKind, RuleFilter, RuleInfo};
use dispatched_core::{EntityPath, Error};
use std::future::Future;
use time::format_description::well_known::Rfc3339;

fn quoted(name: &str, v: &Option<String>) -> Option<String> {
    v.as_ref().map(|v| format!("{name} = '{v}'"))
}

/// One line for a filter: the SQL text, or the correlation fields ANDed.
pub fn filter_text(f: &RuleFilter) -> String {
    match f {
        RuleFilter::Sql { expression } => expression.clone(),
        RuleFilter::Correlation {
            correlation_id,
            message_id,
            to,
            reply_to,
            subject,
            session_id,
            reply_to_session_id,
            content_type,
            properties,
        } => {
            let mut parts: Vec<String> = [
                quoted("CorrelationId", correlation_id),
                quoted("MessageId", message_id),
                quoted("To", to),
                quoted("ReplyTo", reply_to),
                quoted("Subject", subject),
                quoted("SessionId", session_id),
                quoted("ReplyToSessionId", reply_to_session_id),
                quoted("ContentType", content_type),
            ]
            .into_iter()
            .flatten()
            .collect();
            parts.extend(
                properties
                    .iter()
                    .map(|p| format!("{} = '{}'", p.name, p.value)),
            );
            if parts.is_empty() {
                "(matches nothing: no fields set)".into()
            } else {
                parts.join(" AND ")
            }
        }
        RuleFilter::True => "TRUE (matches every message)".into(),
        RuleFilter::Unknown => "unknown filter".into(),
    }
}

pub fn rules_table(rules: &[RuleInfo]) -> String {
    let rows: Vec<Vec<String>> = rules
        .iter()
        .map(|r| {
            vec![
                sanitize(&r.name),
                sanitize(&filter_text(&r.filter)),
                sanitize(r.action.as_deref().unwrap_or("")),
            ]
        })
        .collect();
    table(&["Rule", "Filter", "Action"], &rows)
}

fn time_cell(t: Option<time::OffsetDateTime>) -> String {
    t.and_then(|t| t.format(&Rfc3339).ok()).unwrap_or_default()
}

pub fn sessions_table(l: &SessionListing) -> String {
    let rows: Vec<Vec<String>> = l
        .sessions
        .iter()
        .map(|s: &SessionSummary| {
            let n = match s.messages {
                None => "?".to_string(),
                Some(n) if l.truncated => format!("~{n}+"),
                Some(n) => n.to_string(),
            };
            vec![
                sanitize(&s.id),
                n,
                time_cell(s.first_enqueued),
                time_cell(s.last_enqueued),
            ]
        })
        .collect();
    table(
        &["Session", "Messages", "First enqueued", "Last enqueued"],
        &rows,
    )
}

/// What to tell the user (on stderr) about where the sessions and counts came from.
pub fn sessions_notes(l: &SessionListing) -> Vec<String> {
    let mut notes = Vec::new();
    match &l.broker {
        BrokerList::Listed { capped: false } => {}
        BrokerList::Listed { capped: true } => notes.push(format!(
            "The service's session list was cut at {}; more sessions may exist.",
            thousands(MAX_SESSIONS as u64)
        )),
        BrokerList::Empty => notes.push(format!(
            "The service listed no sessions; showing sessions found in the first {} messages.",
            thousands(l.scanned)
        )),
        BrokerList::Unavailable { .. } => notes.push(
            "Couldn't ask the service for sessions; showing sessions found by peeking.".into(),
        ),
    }
    if l.truncated {
        notes.push(format!(
            "Message counts are lower bounds: only the first {} messages were counted, and there are more.",
            thousands(l.scanned)
        ));
    }
    notes
}

pub fn details_table(d: &EntityDetails) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    if let Some(s) = &d.status {
        rows.push(vec!["Status".into(), sanitize(s)]);
    }
    if let Some(c) = &d.counts {
        let q = if c.truncated { "+" } else { "" };
        let a = if c.approximate { "~" } else { "" };
        for (k, v) in [
            ("Active", c.active),
            ("Dead-letter", c.dead_letter),
            ("Scheduled", c.scheduled),
            ("Transfer", c.transfer),
            ("Transfer DLQ", c.transfer_dead_letter),
        ] {
            rows.push(vec![k.into(), format!("{a}{v}{q}")]);
        }
    }
    rows.extend(
        d.properties
            .iter()
            .filter(|p| p.name != "Status")
            .map(|p| vec![sanitize(&p.name), sanitize(&p.value)]),
    );
    table(&["Property", "Value"], &rows)
}

/// An entity's details: a subscription path is a subscription; any other is a queue, else a
/// topic (only a missing queue falls through; other errors are reported as they are).
pub(crate) async fn details_of<F, Fut>(
    path: &EntityPath,
    mut fetch: F,
) -> dispatched_core::Result<EntityDetails>
where
    F: FnMut(EntityKind) -> Fut,
    Fut: Future<Output = dispatched_core::Result<EntityDetails>>,
{
    if path.subscription.is_some() {
        return fetch(EntityKind::Subscription).await;
    }
    match fetch(EntityKind::Queue).await {
        Err(Error::NotFound(_)) => fetch(EntityKind::Topic).await,
        other => other,
    }
}

pub async fn show(
    ctx: &Ctx,
    sel: &Selection<'_>,
    entity: &str,
    format: Format,
) -> anyhow::Result<()> {
    let path = EntityPath::parse(entity)?;
    let ex = explorer_for(ctx, sel)?;
    let result = details_of(&path, |kind| ex.entity_details(kind, entity)).await;
    ex.close().await;
    let d = result?;
    match format {
        Format::Json => println!("{}", serde_json::to_string_pretty(&d)?),
        _ => println!("{}", details_table(&d)),
    }
    Ok(())
}

pub async fn sessions(
    ctx: &Ctx,
    sel: &Selection<'_>,
    entity: &str,
    format: Format,
) -> anyhow::Result<()> {
    let path = EntityPath::parse(entity)?;
    let ex = explorer_for(ctx, sel)?;
    let result = ex.list_sessions(&path).await;
    ex.close().await;
    let l = result?;
    match format {
        Format::Json => println!("{}", serde_json::to_string_pretty(&l)?),
        _ => {
            println!("{}", sessions_table(&l));
            if l.sessions.is_empty() {
                eprintln!("(no sessions)");
            }
            for n in sessions_notes(&l) {
                eprintln!("{}", sanitize(&n));
            }
        }
    }
    Ok(())
}

/// The topic and subscription of a `<topic>/subscriptions/<subscription>` path.
fn rule_target(subscription: &str) -> anyhow::Result<(String, String)> {
    let path = EntityPath::parse(subscription)?;
    match path.subscription {
        Some(sub) => Ok((path.entity, sub)),
        None => anyhow::bail!(
            "rules belong to a subscription: pass <topic>/subscriptions/<subscription>"
        ),
    }
}

pub async fn rules(
    ctx: &Ctx,
    sel: &Selection<'_>,
    subscription: &str,
    format: Format,
) -> anyhow::Result<()> {
    let (topic, sub) = rule_target(subscription)?;
    let ex = explorer_for(ctx, sel)?;
    let result = ex.list_rules(&topic, &sub).await;
    ex.close().await;
    let rules = result?;
    match format {
        Format::Json => println!("{}", serde_json::to_string_pretty(&rules)?),
        _ => println!("{}", rules_table(&rules)),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::messaging::sessions::BrokerList;
    use dispatched_core::mgmt::{CorrelationProperty, EntityCounts, EntityProperty, RuleFilter};

    fn listing(broker: BrokerList, truncated: bool) -> SessionListing {
        SessionListing {
            sessions: vec![
                SessionSummary {
                    id: "a".into(),
                    messages: Some(2),
                    first_enqueued: None,
                    last_enqueued: None,
                    in_broker: true,
                },
                SessionSummary {
                    id: "b".into(),
                    messages: None,
                    first_enqueued: None,
                    last_enqueued: None,
                    in_broker: true,
                },
            ],
            broker,
            scanned: 10_000,
            truncated,
        }
    }

    #[test]
    fn rules_table_shows_sql_and_correlation_filters() {
        let rules = vec![
            RuleInfo {
                name: "$Default".into(),
                filter: RuleFilter::Sql {
                    expression: "1=1".into(),
                },
                action: None,
                created_at: None,
            },
            RuleInfo {
                name: "priority".into(),
                filter: RuleFilter::Correlation {
                    correlation_id: Some("batch-7".into()),
                    message_id: None,
                    to: None,
                    reply_to: None,
                    subject: Some("priority".into()),
                    session_id: None,
                    reply_to_session_id: None,
                    content_type: None,
                    properties: vec![CorrelationProperty {
                        name: "region".into(),
                        value: "eu".into(),
                    }],
                },
                action: Some("SET x = 1".into()),
                created_at: None,
            },
            RuleInfo {
                name: "odd".into(),
                filter: RuleFilter::Unknown,
                action: None,
                created_at: None,
            },
        ];
        let t = rules_table(&rules);
        assert!(t.contains("$Default") && t.contains("1=1"), "{t}");
        assert!(
            t.contains("CorrelationId = 'batch-7'")
                && t.contains("Subject = 'priority'")
                && t.contains("region = 'eu'"),
            "{t}"
        );
        assert!(t.contains("SET x = 1"));
        assert!(t.contains("unknown filter"), "{t}");
    }

    #[test]
    fn sessions_table_marks_lower_bounds_and_unknowns() {
        let l = listing(BrokerList::Listed { capped: false }, true);
        let t = sessions_table(&l);
        assert!(t.contains("~2+"), "{t}");
        assert!(t.contains('?'));
        let exact = sessions_table(&listing(BrokerList::Listed { capped: false }, false));
        assert_eq!(
            exact.lines().nth(1).unwrap().split_whitespace().nth(1),
            Some("2"),
            "{exact}"
        );
        assert!(!exact.contains('~'), "{exact}");
        let notes = sessions_notes(&l);
        assert!(
            notes
                .iter()
                .any(|n| n.contains("lower bounds") && n.contains("first 10,000")),
            "{notes:?}"
        );
    }

    #[test]
    fn session_notes_say_what_the_service_contributed() {
        let none = |l: &SessionListing| sessions_notes(l);
        assert!(none(&listing(BrokerList::Listed { capped: false }, false)).is_empty());
        let capped = none(&listing(BrokerList::Listed { capped: true }, false));
        assert!(
            capped.len() == 1 && capped[0].contains("10,000"),
            "{capped:?}"
        );
        let empty = none(&listing(BrokerList::Empty, false));
        assert_eq!(
            empty,
            vec![
                "The service listed no sessions; showing sessions found in the first 10,000 messages."
            ]
        );
        let down = none(&listing(
            BrokerList::Unavailable {
                kind: "network".into(),
            },
            false,
        ));
        assert_eq!(
            down,
            vec!["Couldn't ask the service for sessions; showing sessions found by peeking."]
        );
        // Both notes can apply.
        assert_eq!(none(&listing(BrokerList::Empty, true)).len(), 2);
    }

    fn counted(approximate: bool, truncated: bool) -> EntityDetails {
        EntityDetails {
            kind: EntityKind::Queue,
            name: "q".into(),
            topic: None,
            status: None,
            counts: Some(EntityCounts {
                active: 5,
                dead_letter: 4,
                scheduled: 3,
                transfer: 2,
                transfer_dead_letter: 1,
                approximate,
                truncated,
            }),
            size_in_bytes: None,
            properties: vec![],
        }
    }

    #[test]
    fn details_table_counts_are_exact_approximate_or_lower_bounds() {
        let cells = |d: &EntityDetails| -> Vec<(String, String)> {
            details_table(d)
                .lines()
                .skip(1)
                .map(|l| {
                    let (k, v) = l.rsplit_once("  ").unwrap();
                    (k.trim().to_string(), v.trim().to_string())
                })
                .collect()
        };
        let exact = cells(&counted(false, false));
        assert_eq!(exact.len(), 5, "{exact:?}");
        assert_eq!(exact[0], ("Active".to_string(), "5".to_string()));
        assert_eq!(exact[4], ("Transfer DLQ".to_string(), "1".to_string()));
        let approx = cells(&counted(true, false));
        assert!(
            approx
                .iter()
                .all(|(_, v)| v.starts_with('~') && !v.ends_with('+'))
        );
        assert_eq!(approx[1], ("Dead-letter".to_string(), "~4".to_string()));
        let lower = cells(&counted(true, true));
        assert_eq!(lower[2], ("Scheduled".to_string(), "~3+".to_string()));
        assert!(
            lower
                .iter()
                .all(|(_, v)| v.starts_with('~') && v.ends_with('+'))
        );
    }

    fn details(kind: EntityKind) -> EntityDetails {
        EntityDetails {
            kind,
            name: "x".into(),
            topic: None,
            status: None,
            counts: None,
            size_in_bytes: None,
            properties: vec![],
        }
    }

    #[tokio::test]
    async fn show_tries_a_queue_then_a_topic_and_keeps_other_errors() {
        use std::sync::Mutex;
        let asked = Mutex::new(Vec::new());
        let missing_queue = |k: EntityKind| {
            asked.lock().unwrap().push(k);
            async move {
                match k {
                    EntityKind::Queue => Err(Error::NotFound("queue `x`".into())),
                    k => Ok(details(k)),
                }
            }
        };
        let d = details_of(&EntityPath::queue("x"), missing_queue)
            .await
            .unwrap();
        assert_eq!(d.kind, EntityKind::Topic);
        assert_eq!(
            *asked.lock().unwrap(),
            [EntityKind::Queue, EntityKind::Topic]
        );

        // A queue is not followed by a topic lookup.
        let asked = Mutex::new(Vec::new());
        let d = details_of(&EntityPath::queue("x"), |k| {
            asked.lock().unwrap().push(k);
            async move { Ok(details(k)) }
        })
        .await
        .unwrap();
        assert_eq!(d.kind, EntityKind::Queue);
        assert_eq!(*asked.lock().unwrap(), [EntityKind::Queue]);

        // Only a missing queue falls through: other errors are reported as they are.
        let e = details_of(&EntityPath::queue("x"), |_| async {
            Err::<EntityDetails, _>(Error::Network("down".into()))
        })
        .await
        .unwrap_err();
        assert!(matches!(e, Error::Network(_)), "{e}");

        // A subscription path asks for a subscription only.
        let asked = Mutex::new(Vec::new());
        let sub = EntityPath::subscription("t", "s");
        let e = details_of(&sub, |k| {
            asked.lock().unwrap().push(k);
            async { Err::<EntityDetails, _>(Error::NotFound("subscription".into())) }
        })
        .await
        .unwrap_err();
        assert!(matches!(e, Error::NotFound(_)));
        assert_eq!(*asked.lock().unwrap(), [EntityKind::Subscription]);
    }

    #[test]
    fn rules_need_a_subscription_path() {
        let (topic, sub) = rule_target("orders/eu/subscriptions/audit").unwrap();
        assert_eq!((topic.as_str(), sub.as_str()), ("orders/eu", "audit"));
        let e = rule_target("orders").unwrap_err().to_string();
        assert!(e.contains("<topic>/subscriptions/<subscription>"), "{e}");
        assert!(rule_target("a/subscriptions/").is_err());
    }

    #[test]
    fn details_table_lists_properties_with_counts_first() {
        let d = EntityDetails {
            kind: EntityKind::Queue,
            name: "orders".into(),
            topic: None,
            status: Some("Active".into()),
            counts: None,
            size_in_bytes: Some(81_920),
            properties: vec![EntityProperty {
                name: "LockDuration".into(),
                value: "PT1M".into(),
            }],
        };
        let t = details_table(&d);
        assert!(t.lines().next().unwrap().contains("Property"));
        assert!(
            t.contains("LockDuration") && t.contains("PT1M") && t.contains("Status"),
            "{t}"
        );
    }
}
