//! Subscription rules (read-only in M1c).

use super::xml::Node;
use serde::Serialize;

/// A user property a correlation filter matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CorrelationProperty {
    /// Property name.
    pub name: String,
    /// Expected value as text.
    pub value: String,
}

/// A rule's filter. `TrueFilter`/`FalseFilter` arrive as SQL `1=1` / `1=0`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RuleFilter {
    /// SQL filter expression.
    Sql {
        /// The expression.
        expression: String,
    },
    /// Correlation filter: every set field must equal the message's.
    Correlation {
        /// Expected `correlation-id`.
        correlation_id: Option<String>,
        /// Expected `message-id`.
        message_id: Option<String>,
        /// Expected `to`.
        to: Option<String>,
        /// Expected `reply-to`.
        reply_to: Option<String>,
        /// Expected subject (`Label` on the wire).
        subject: Option<String>,
        /// Expected `session-id`.
        session_id: Option<String>,
        /// Expected `reply-to-session-id`.
        reply_to_session_id: Option<String>,
        /// Expected `content-type`.
        content_type: Option<String>,
        /// Expected user properties.
        properties: Vec<CorrelationProperty>,
    },
    /// A filter this reader does not understand (missing, empty or of a type it does not know).
    /// Shown as such rather than guessed at; the XML reader does not keep the `i:type` name.
    Unknown,
}

/// A rule of a subscription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleInfo {
    /// Rule name.
    pub name: String,
    /// Its filter.
    pub filter: RuleFilter,
    /// SQL action, if any.
    pub action: Option<String>,
    /// Creation time as the service wrote it.
    pub created_at: Option<String>,
}

fn text(n: &Node, name: &str) -> Option<String> {
    n.text_of(name).map(str::to_string)
}

fn correlation(f: &Node) -> RuleFilter {
    let properties: Vec<CorrelationProperty> = f
        .child("Properties")
        .map(|p| {
            p.children_named("KeyValueOfstringanyType")
                .filter_map(|kv| {
                    Some(CorrelationProperty {
                        name: kv.text_of("Key")?.to_string(),
                        value: kv
                            .child("Value")
                            .map(|v| v.text.clone())
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let rule = RuleFilter::Correlation {
        correlation_id: text(f, "CorrelationId"),
        message_id: text(f, "MessageId"),
        to: text(f, "To"),
        reply_to: text(f, "ReplyTo"),
        subject: text(f, "Label"),
        session_id: text(f, "SessionId"),
        reply_to_session_id: text(f, "ReplyToSessionId"),
        content_type: text(f, "ContentType"),
        properties,
    };
    match &rule {
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
        } if properties.is_empty() => RuleFilter::Unknown,
        _ => rule,
    }
}

pub(crate) fn rule_from_entry(entry: &Node) -> Option<RuleInfo> {
    let name = entry.text_of("title")?.to_string();
    let d = entry.path(&["content", "RuleDescription"])?;
    // The element's i:type attribute is not kept by the XML reader; SQL-shaped filters
    // (SqlFilter, TrueFilter, FalseFilter) carry SqlExpression, correlation filters carry
    // correlation fields. Anything else is unknown, never guessed.
    let filter = match d.child("Filter") {
        None => RuleFilter::Unknown,
        Some(f) => match f.text_of("SqlExpression") {
            Some(expression) => RuleFilter::Sql {
                expression: expression.to_string(),
            },
            None => correlation(f),
        },
    };
    Some(RuleInfo {
        name,
        filter,
        action: d
            .child("Action")
            .and_then(|a| a.text_of("SqlExpression"))
            .map(str::to_string),
        created_at: text(d, "CreatedAt"),
    })
}
