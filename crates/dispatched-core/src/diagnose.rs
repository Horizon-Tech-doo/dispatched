//! Why messages may not arrive at a subscription: plain sentences from its description, its
//! topic's status, its rules, its counts and its dead-letter reasons.
//!
//! The analysis is pure: a surface gathers [`SubscriptionFacts`] (management reads and a peek of
//! the dead-letter queue) and [`diagnose_subscription`] explains them. Nothing here does I/O.

use crate::messaging::{ReasonGroup, duration_text};
use crate::mgmt::{
    EntityCounts, EntityStatus, RuleFilter, RuleInfo, SubscriptionSettings, TimeSpan,
};
use serde::Serialize;
use std::time::Duration;

/// How much a finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Worth knowing; not a cause on its own.
    Info,
    /// Can keep messages away under some conditions.
    Warning,
    /// Keeps messages away.
    Problem,
}

/// One explained fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// How much it matters.
    pub severity: Severity,
    /// The explanation, in one or two sentences.
    pub text: String,
}

/// What is known about a subscription. Every part that could not be read is `None`, and the
/// analysis says so instead of guessing.
#[derive(Debug, Clone, Default)]
pub struct SubscriptionFacts {
    /// The topic.
    pub topic: String,
    /// The subscription name.
    pub subscription: String,
    /// The topic's status.
    pub topic_status: Option<EntityStatus>,
    /// The subscription's settings.
    pub settings: SubscriptionSettings,
    /// Its rules; `None` when they could not be listed.
    pub rules: Option<Vec<RuleInfo>>,
    /// Its counts; `None` when not reported.
    pub counts: Option<EntityCounts>,
    /// Dead-lettered messages grouped by reason (from a capped peek); empty when not read.
    pub dead_letter_reasons: Vec<ReasonGroup>,
}

/// A lock this short is easily outlived by a consumer.
const SHORT_LOCK: Duration = Duration::from_secs(15);
/// A delivery count this low dead-letters a message after a few failures.
const LOW_MAX_DELIVERY: u32 = 3;
/// A time to live this short expires messages before a slow consumer gets to them.
const SHORT_TTL: Duration = Duration::from_secs(300);

/// The reason the broker gives a message dead-lettered after too many deliveries.
const MAX_DELIVERY_REASON: &str = "MaxDeliveryCountExceeded";
/// The reason the broker gives an expired message.
const EXPIRED_REASON: &str = "TTLExpiredException";

/// Whether a SQL filter can never be true (`1=0`, `FALSE` and the like).
fn never_true(expression: &str) -> bool {
    let compact: String = expression
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(
        compact.trim_start_matches('(').trim_end_matches(')'),
        "1=0" | "0=1" | "false" | "1<>1" | "1!=1" | "nottrue"
    )
}

/// Whether a rule lets no message through, with the reason.
fn blocks_everything(rule: &RuleInfo) -> Option<String> {
    match &rule.filter {
        RuleFilter::Sql { expression } if never_true(expression) => Some(format!(
            "Rule {} has the filter `{}`, which is never true, so it lets no message through.",
            rule.name,
            expression.trim()
        )),
        RuleFilter::Correlation {
            correlation_id: None,
            message_id: None,
            to: None,
            reply_to: None,
            subject: None,
            session_id: None,
            reply_to_session_id: None,
            content_type: None,
            properties,
        } if properties.is_empty() => Some(format!(
            "Rule {} is a correlation filter with no field set, so it matches no message.",
            rule.name
        )),
        _ => None,
    }
}

/// One line describing a rule that may let messages through.
fn describe_rule(rule: &RuleInfo) -> Finding {
    let (severity, text) = match &rule.filter {
        RuleFilter::True => (
            Severity::Info,
            format!("Rule {} matches every message.", rule.name),
        ),
        RuleFilter::Sql { expression } => (
            Severity::Info,
            format!(
                "Rule {} has the SQL filter `{}`: only messages for which it is true arrive. \
                 Check that senders set the properties it reads, with the same names and types.",
                rule.name,
                expression.trim()
            ),
        ),
        RuleFilter::Correlation { .. } => (
            Severity::Info,
            format!(
                "Rule {} is a correlation filter: only messages whose fields equal every value \
                 it sets arrive (exact, case-sensitive matches).",
                rule.name
            ),
        ),
        RuleFilter::Unknown => (
            Severity::Warning,
            format!(
                "Rule {} has a filter Dispatched cannot read; check it in the Azure portal.",
                rule.name
            ),
        ),
    };
    Finding { severity, text }
}

fn status_finding(what: &str, status: EntityStatus) -> Option<Finding> {
    let (severity, text) = match status {
        EntityStatus::Active => return None,
        EntityStatus::Disabled => (
            Severity::Problem,
            format!("{what} is disabled: nothing is sent to it or received from it."),
        ),
        EntityStatus::SendDisabled => (
            Severity::Problem,
            format!("{what} is send-disabled: new messages are refused."),
        ),
        EntityStatus::ReceiveDisabled => (
            Severity::Problem,
            format!("{what} is receive-disabled: messages are kept but no receiver gets them."),
        ),
    };
    Some(Finding { severity, text })
}

fn count_words(n: u64, approximate: bool) -> String {
    let n = crate::safety::count_text(n);
    if approximate { format!("about {n}") } else { n }
}

fn messages_word(n: u64) -> &'static str {
    if n == 1 { "message" } else { "messages" }
}

/// Explain why messages may not arrive at the subscription. Problems come first, then
/// warnings, then information, each in the order checked.
pub fn diagnose_subscription(f: &SubscriptionFacts) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    let mut push = |severity: Severity, text: String| out.push(Finding { severity, text });
    let s = &f.settings;
    let sub = format!("{}/subscriptions/{}", f.topic, f.subscription);

    // Status of the topic, then of the subscription.
    if let Some(found) = f
        .topic_status
        .and_then(|st| status_finding(&format!("The topic {}", f.topic), st))
    {
        push(found.severity, found.text);
    }
    if let Some(found) = s
        .status
        .and_then(|st| status_finding(&format!("The subscription {sub}"), st))
    {
        push(found.severity, found.text);
    }

    // Forwarding: messages leave the subscription as they arrive.
    if let Some(to) = s.forward_to.as_deref().filter(|t| !t.trim().is_empty()) {
        push(
            Severity::Warning,
            format!(
                "Messages are forwarded to {to} as they arrive (ForwardTo), so they never stay \
                 in this subscription; receive from {to} instead."
            ),
        );
    }
    if let Some(to) = s
        .forward_dead_lettered_messages_to
        .as_deref()
        .filter(|t| !t.trim().is_empty())
    {
        push(
            Severity::Info,
            format!(
                "Dead-lettered messages are forwarded to {to} \
                 (ForwardDeadLetteredMessagesTo), so this subscription's dead-letter queue stays \
                 empty; look for them in {to}."
            ),
        );
    }

    // Rules.
    match &f.rules {
        None => push(
            Severity::Info,
            "The rules could not be read, so the filters were not checked.".into(),
        ),
        Some(rules) if rules.is_empty() => push(
            Severity::Problem,
            "The subscription has no rules, so no message from the topic matches it. Add a rule \
             (the SQL filter `1=1` matches every message)."
                .into(),
        ),
        Some(rules) => {
            let blocked: Vec<String> = rules.iter().filter_map(blocks_everything).collect();
            if blocked.len() == rules.len() {
                for b in blocked {
                    push(Severity::Problem, b);
                }
                push(
                    Severity::Problem,
                    "No rule lets a message through, so nothing sent to the topic arrives here."
                        .into(),
                );
            } else {
                for b in blocked {
                    push(Severity::Info, b);
                }
                for r in rules.iter().filter(|r| blocks_everything(r).is_none()) {
                    let d = describe_rule(r);
                    push(d.severity, d.text);
                }
            }
        }
    }

    // Sessions.
    if s.requires_session == Some(true) {
        push(
            Severity::Info,
            "The subscription requires sessions: a receiver must accept a session (by id or the \
             next available one); a plain receiver gets nothing, and messages sent without a \
             SessionId are refused."
                .into(),
        );
    }

    // Counts.
    let dead_letter = match &f.counts {
        None => {
            push(
                Severity::Info,
                "Message counts are not available, so the queue depth was not checked.".into(),
            );
            None
        }
        Some(c) => {
            let approx = c.approximate || c.truncated;
            if c.active > 0 {
                push(
                    Severity::Info,
                    format!(
                        "{} active {} wait in the subscription: they arrive, but are not being \
                         received. Check that a consumer is running and listening to {sub}.",
                        count_words(c.active, approx),
                        messages_word(c.active)
                    ),
                );
            }
            if c.dead_letter > 0 {
                push(
                    if c.active == 0 {
                        Severity::Warning
                    } else {
                        Severity::Info
                    },
                    format!(
                        "{} {} are in the dead-letter queue{}. Look at get_dlq_summary for the \
                         reasons.",
                        count_words(c.dead_letter, approx),
                        messages_word(c.dead_letter),
                        if c.active == 0 {
                            " and none are active: messages arrive but end up dead-lettered"
                        } else {
                            ""
                        }
                    ),
                );
            }
            if c.active == 0 && c.dead_letter == 0 && c.scheduled == 0 {
                push(
                    Severity::Info,
                    "The subscription is empty: either nothing reached it, or a consumer takes \
                     every message."
                        .into(),
                );
            }
            Some(c.dead_letter)
        }
    };

    // Delivery count and lock duration.
    let by_reason = |reason: &str| -> u64 {
        f.dead_letter_reasons
            .iter()
            .filter(|g| g.reason.as_deref() == Some(reason))
            .map(|g| g.count)
            .sum()
    };
    let exceeded = by_reason(MAX_DELIVERY_REASON);
    let expired = by_reason(EXPIRED_REASON);
    let short_lock = s.lock_duration.is_some_and(|l| l <= SHORT_LOCK);
    let low_max = s.max_delivery_count.is_some_and(|m| m <= LOW_MAX_DELIVERY);
    let lock = s.lock_duration.map(duration_text);
    let max = s.max_delivery_count;
    if exceeded > 0 {
        push(
            Severity::Problem,
            format!(
                "{} dead-lettered {} reached MaxDeliveryCount{}: consumers abandon them, fail \
                 on them, or take longer than the lock{}. Fix the consumer, or raise \
                 LockDuration or MaxDeliveryCount.",
                crate::safety::count_text(exceeded),
                messages_word(exceeded),
                max.map(|m| format!(" ({m})")).unwrap_or_default(),
                lock.as_deref()
                    .map(|l| format!(" (LockDuration {l})"))
                    .unwrap_or_default()
            ),
        );
    } else if short_lock && low_max {
        push(
            Severity::Warning,
            format!(
                "LockDuration is {} and MaxDeliveryCount is {}: a consumer slower than the lock \
                 loses it, and after {} such deliveries the message is dead-lettered{}.",
                lock.as_deref().unwrap_or("?"),
                max.unwrap_or_default(),
                max.unwrap_or_default(),
                if dead_letter.unwrap_or(0) > 0 {
                    ", which matches the dead-letter queue growing"
                } else {
                    ""
                }
            ),
        );
    } else {
        if short_lock {
            push(
                Severity::Warning,
                format!(
                    "LockDuration is {}: a consumer that takes longer loses the lock, the \
                     message is delivered again and its delivery count rises.",
                    lock.as_deref().unwrap_or("?")
                ),
            );
        }
        if low_max {
            push(
                Severity::Warning,
                format!(
                    "MaxDeliveryCount is {}: a message is dead-lettered after {} failed or \
                     abandoned deliveries.",
                    max.unwrap_or_default(),
                    max.unwrap_or_default()
                ),
            );
        }
    }
    if !short_lock && !low_max && (lock.is_some() || max.is_some()) {
        push(
            Severity::Info,
            format!(
                "LockDuration is {} and MaxDeliveryCount is {}.",
                lock.as_deref().unwrap_or("not reported"),
                max.map(|m| m.to_string())
                    .unwrap_or_else(|| "not reported".into())
            ),
        );
    }

    // Time to live and dead-lettering on expiry.
    let dead_letters_expired = s.dead_lettering_on_message_expiration == Some(true);
    match s.default_message_time_to_live {
        Some(ttl) if ttl != TimeSpan::MAX => {
            let text = duration_text(ttl);
            let fate = if dead_letters_expired {
                "expired messages go to the dead-letter queue"
            } else {
                "expired messages are dropped without a trace (DeadLetteringOnMessageExpiration \
                 is off)"
            };
            let severity = if ttl <= SHORT_TTL {
                Severity::Warning
            } else {
                Severity::Info
            };
            push(
                severity,
                format!(
                    "Messages expire {text} after they are enqueued unless the sender sets a \
                     shorter time to live; {fate}."
                ),
            );
        }
        _ => {}
    }
    if expired > 0 {
        push(
            Severity::Warning,
            format!(
                "{} dead-lettered {} expired before a consumer received {}.",
                crate::safety::count_text(expired),
                messages_word(expired),
                if expired == 1 { "it" } else { "them" }
            ),
        );
    }

    out.sort_by(|a, b| b.severity.cmp(&a.severity));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mgmt::CorrelationProperty;

    fn rule(name: &str, filter: RuleFilter) -> RuleInfo {
        RuleInfo {
            name: name.into(),
            filter,
            action: None,
            created_at: None,
        }
    }

    fn sql(e: &str) -> RuleFilter {
        RuleFilter::Sql {
            expression: e.into(),
        }
    }

    fn empty_correlation() -> RuleFilter {
        RuleFilter::Correlation {
            correlation_id: None,
            message_id: None,
            to: None,
            reply_to: None,
            subject: None,
            session_id: None,
            reply_to_session_id: None,
            content_type: None,
            properties: vec![],
        }
    }

    fn counts(active: u64, dead_letter: u64) -> EntityCounts {
        EntityCounts {
            active,
            dead_letter,
            ..EntityCounts::default()
        }
    }

    /// A healthy subscription: active, a `$Default` true rule, normal lock and delivery count.
    fn healthy() -> SubscriptionFacts {
        SubscriptionFacts {
            topic: "events".into(),
            subscription: "billing".into(),
            topic_status: Some(EntityStatus::Active),
            settings: SubscriptionSettings {
                status: Some(EntityStatus::Active),
                lock_duration: Some(Duration::from_secs(60)),
                max_delivery_count: Some(10),
                default_message_time_to_live: Some(TimeSpan::MAX),
                ..SubscriptionSettings::default()
            },
            rules: Some(vec![rule("$Default", RuleFilter::True)]),
            counts: Some(counts(0, 0)),
            dead_letter_reasons: vec![],
        }
    }

    fn with(sev: Severity, findings: &[Finding], needle: &str) -> bool {
        findings
            .iter()
            .any(|f| f.severity == sev && f.text.contains(needle))
    }

    fn worst(findings: &[Finding]) -> Severity {
        findings
            .iter()
            .map(|f| f.severity)
            .max()
            .unwrap_or(Severity::Info)
    }

    #[test]
    fn a_healthy_subscription_has_only_information() {
        let f = diagnose_subscription(&healthy());
        assert_eq!(worst(&f), Severity::Info, "{f:?}");
        assert!(with(Severity::Info, &f, "matches every message"), "{f:?}");
        assert!(with(Severity::Info, &f, "is empty"), "{f:?}");
        assert!(with(Severity::Info, &f, "LockDuration is 1m"), "{f:?}");
    }

    #[test]
    fn disabled_topic_and_receive_disabled_subscription_are_problems() {
        let mut facts = healthy();
        facts.topic_status = Some(EntityStatus::Disabled);
        facts.settings.status = Some(EntityStatus::ReceiveDisabled);
        let f = diagnose_subscription(&facts);
        assert!(
            with(Severity::Problem, &f, "topic events is disabled"),
            "{f:?}"
        );
        assert!(with(Severity::Problem, &f, "receive-disabled"), "{f:?}");
        assert_eq!(f[0].severity, Severity::Problem, "problems come first");
    }

    #[test]
    fn forwarding_says_where_messages_go() {
        let mut facts = healthy();
        facts.settings.forward_to = Some("orders".into());
        facts.settings.forward_dead_lettered_messages_to = Some("dlq-sink".into());
        let f = diagnose_subscription(&facts);
        assert!(with(Severity::Warning, &f, "forwarded to orders"), "{f:?}");
        assert!(with(Severity::Info, &f, "dlq-sink"), "{f:?}");
        // An empty target is no forwarding.
        let mut facts = healthy();
        facts.settings.forward_to = Some(String::new());
        assert!(!with(
            Severity::Warning,
            &diagnose_subscription(&facts),
            "forwarded"
        ));
    }

    #[test]
    fn rules_that_match_nothing_are_problems() {
        let mut facts = healthy();
        facts.rules = Some(vec![]);
        let f = diagnose_subscription(&facts);
        assert!(with(Severity::Problem, &f, "no rules"), "{f:?}");

        facts.rules = Some(vec![
            rule("$Default", sql(" 1 = 0 ")),
            rule("none", empty_correlation()),
        ]);
        let f = diagnose_subscription(&facts);
        assert!(
            with(Severity::Problem, &f, "`1 = 0`, which is never true"),
            "{f:?}"
        );
        assert!(with(Severity::Problem, &f, "no field set"), "{f:?}");
        assert!(
            with(Severity::Problem, &f, "No rule lets a message through"),
            "{f:?}"
        );

        facts.rules = Some(vec![rule("off", sql("FALSE"))]);
        assert!(with(
            Severity::Problem,
            &diagnose_subscription(&facts),
            "never true"
        ));
    }

    #[test]
    fn other_filters_are_listed_and_a_blocked_rule_beside_them_is_information() {
        let mut facts = healthy();
        facts.rules = Some(vec![
            rule("eu", sql("region = 'eu'")),
            rule(
                "prio",
                RuleFilter::Correlation {
                    correlation_id: None,
                    message_id: None,
                    to: None,
                    reply_to: None,
                    subject: Some("priority".into()),
                    session_id: None,
                    reply_to_session_id: None,
                    content_type: None,
                    properties: vec![CorrelationProperty {
                        name: "k".into(),
                        value: "v".into(),
                    }],
                },
            ),
            rule("off", sql("1=0")),
            rule("odd", RuleFilter::Unknown),
        ]);
        let f = diagnose_subscription(&facts);
        assert!(with(Severity::Info, &f, "`region = 'eu'`"), "{f:?}");
        assert!(
            with(Severity::Info, &f, "Rule prio is a correlation filter"),
            "{f:?}"
        );
        assert!(
            with(Severity::Info, &f, "Rule off has the filter `1=0`"),
            "{f:?}"
        );
        assert!(with(Severity::Warning, &f, "cannot read"), "{f:?}");
        assert!(!with(Severity::Problem, &f, "No rule"), "{f:?}");
    }

    #[test]
    fn rules_that_cannot_be_read_are_said_so() {
        let mut facts = healthy();
        facts.rules = None;
        assert!(with(
            Severity::Info,
            &diagnose_subscription(&facts),
            "could not be read"
        ));
    }

    #[test]
    fn counts_explain_waiting_and_dead_lettered_messages() {
        let mut facts = healthy();
        facts.counts = Some(counts(12, 0));
        let f = diagnose_subscription(&facts);
        assert!(with(Severity::Info, &f, "12 active messages wait"), "{f:?}");

        facts.counts = Some(EntityCounts {
            dead_letter: 1_284,
            approximate: true,
            ..EntityCounts::default()
        });
        let f = diagnose_subscription(&facts);
        assert!(
            with(
                Severity::Warning,
                &f,
                "about 1,284 messages are in the dead-letter queue and none are active"
            ),
            "{f:?}"
        );

        facts.counts = None;
        assert!(with(
            Severity::Info,
            &diagnose_subscription(&facts),
            "counts are not available"
        ));
    }

    #[test]
    fn low_lock_and_delivery_count_explain_dead_letter_growth() {
        let mut facts = healthy();
        facts.settings.lock_duration = Some(Duration::from_secs(5));
        facts.settings.max_delivery_count = Some(2);
        facts.counts = Some(counts(0, 40));
        let f = diagnose_subscription(&facts);
        assert!(
            with(
                Severity::Warning,
                &f,
                "LockDuration is 5s and MaxDeliveryCount is 2"
            ) && with(Severity::Warning, &f, "dead-letter queue growing"),
            "{f:?}"
        );

        facts.dead_letter_reasons = vec![ReasonGroup {
            reason: Some(MAX_DELIVERY_REASON.into()),
            description: None,
            count: 40,
            first_sequence: 1,
            last_sequence: 40,
            oldest: None,
            newest: None,
        }];
        let f = diagnose_subscription(&facts);
        assert!(
            with(
                Severity::Problem,
                &f,
                "40 dead-lettered messages reached MaxDeliveryCount (2)"
            ) && with(Severity::Problem, &f, "LockDuration 5s"),
            "{f:?}"
        );
    }

    #[test]
    fn ttl_and_expiry_and_sessions() {
        let mut facts = healthy();
        facts.settings.default_message_time_to_live = Some(Duration::from_secs(60));
        facts.settings.dead_lettering_on_message_expiration = Some(false);
        facts.settings.requires_session = Some(true);
        let f = diagnose_subscription(&facts);
        assert!(
            with(Severity::Warning, &f, "expire 1m after")
                && with(Severity::Warning, &f, "dropped without a trace"),
            "{f:?}"
        );
        assert!(with(Severity::Info, &f, "requires sessions"), "{f:?}");

        facts.settings.dead_lettering_on_message_expiration = Some(true);
        facts.dead_letter_reasons = vec![ReasonGroup {
            reason: Some(EXPIRED_REASON.into()),
            description: None,
            count: 1,
            first_sequence: 3,
            last_sequence: 3,
            oldest: None,
            newest: None,
        }];
        let f = diagnose_subscription(&facts);
        assert!(
            with(Severity::Warning, &f, "go to the dead-letter queue"),
            "{f:?}"
        );
        assert!(
            with(Severity::Warning, &f, "1 dead-lettered message expired"),
            "{f:?}"
        );
        // "never" is not mentioned.
        facts.settings.default_message_time_to_live = Some(TimeSpan::MAX);
        assert!(!with(
            Severity::Warning,
            &diagnose_subscription(&facts),
            "expire "
        ));
    }
}
