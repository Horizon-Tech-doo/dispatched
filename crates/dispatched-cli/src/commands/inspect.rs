//! `dispatched show`, `session ls`, `rule ls`: read-only inspection.

use crate::Format;
use crate::commands::browse::explorer_for;
use crate::context::Ctx;
use crate::output::{sanitize, table};
use crate::resolve::Selection;
use dispatched_core::messaging::sessions::{
    BrokerList, MAX_SESSIONS, SessionListing, SessionSummary,
};
use dispatched_core::mgmt::{EntityDetails, EntityKind, RuleFilter, RuleInfo};
use dispatched_core::{EntityPath, Error};
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
            l.scanned
        )),
        BrokerList::Unavailable { .. } => notes.push(
            "Couldn't ask the service for sessions; showing sessions found by peeking.".into(),
        ),
    }
    if l.truncated {
        notes.push(format!(
            "Message counts are lower bounds: only the first {} messages were counted, and there are more.",
            l.scanned
        ));
    }
    notes
}

/// `10000` as `10,000`.
fn thousands(n: u64) -> String {
    let digits: Vec<char> = n.to_string().chars().rev().collect();
    let groups: Vec<String> = digits
        .chunks(3)
        .map(|g| g.iter().rev().collect::<String>())
        .collect();
    groups.iter().rev().cloned().collect::<Vec<_>>().join(",")
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

pub async fn show(
    ctx: &Ctx,
    sel: &Selection<'_>,
    entity: &str,
    format: Format,
) -> anyhow::Result<()> {
    let path = EntityPath::parse(entity)?;
    let ex = explorer_for(ctx, sel)?;
    let result = if path.subscription.is_some() {
        ex.entity_details(EntityKind::Subscription, entity).await
    } else {
        match ex.entity_details(EntityKind::Queue, entity).await {
            Err(Error::NotFound(_)) => ex.entity_details(EntityKind::Topic, entity).await,
            other => other,
        }
    };
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

pub async fn rules(
    ctx: &Ctx,
    sel: &Selection<'_>,
    subscription: &str,
    format: Format,
) -> anyhow::Result<()> {
    let path = EntityPath::parse(subscription)?;
    let Some(sub) = path.subscription.as_deref() else {
        anyhow::bail!("rules belong to a subscription: pass <topic>/subscriptions/<subscription>");
    };
    let ex = explorer_for(ctx, sel)?;
    let result = ex.list_rules(&path.entity, sub).await;
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
    use dispatched_core::mgmt::{CorrelationProperty, EntityProperty, RuleFilter};

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
                .any(|n| n.contains("lower bounds") && n.contains("first 10000")),
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
                "The service listed no sessions; showing sessions found in the first 10000 messages."
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

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(10_000), "10,000");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_234_567), "1,234,567");
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
