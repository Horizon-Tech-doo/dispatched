//! One entity's description: every property the service reports, for an Overview.

use super::models::{EntityCounts, counts};
use super::xml::Node;
use serde::{Deserialize, Serialize};

/// Queue, topic or subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityKind {
    /// A queue.
    Queue,
    /// A topic.
    Topic,
    /// A topic subscription.
    Subscription,
}

impl EntityKind {
    fn element(self) -> &'static str {
        match self {
            EntityKind::Queue => "QueueDescription",
            EntityKind::Topic => "TopicDescription",
            EntityKind::Subscription => "SubscriptionDescription",
        }
    }

    /// `queue`, `topic`, `subscription`.
    pub fn as_str(self) -> &'static str {
        match self {
            EntityKind::Queue => "queue",
            EntityKind::Topic => "topic",
            EntityKind::Subscription => "subscription",
        }
    }
}

/// A property as the service reports it (element name and text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntityProperty {
    /// Element name, e.g. `LockDuration`.
    pub name: String,
    /// Text, e.g. `PT1M`.
    pub value: String,
}

/// An entity's description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntityDetails {
    /// What it is.
    pub kind: EntityKind,
    /// Its name (the subscription name for a subscription).
    pub name: String,
    /// The topic of a subscription.
    pub topic: Option<String>,
    /// `Active`, `Disabled`, `SendDisabled`, `ReceiveDisabled`.
    pub status: Option<String>,
    /// Counts; `None` when not reported (topics, the emulator).
    pub counts: Option<EntityCounts>,
    /// Size in bytes.
    pub size_in_bytes: Option<u64>,
    /// Every other non-empty leaf, in document order.
    pub properties: Vec<EntityProperty>,
}

/// Nested parts that are not plain properties (counts are their own field; rules hold key names).
const SKIPPED: &[&str] = &["CountDetails", "AuthorizationRules"];

pub(crate) fn details_from_entry(
    kind: EntityKind,
    topic: Option<&str>,
    entry: &Node,
) -> Option<EntityDetails> {
    let name = entry.text_of("title")?.to_string();
    let d = entry.path(&["content", kind.element()])?;
    let properties = d
        .children
        .iter()
        .filter(|c| {
            c.children.is_empty() && !c.text.is_empty() && !SKIPPED.contains(&c.name.as_str())
        })
        .map(|c| EntityProperty {
            name: c.name.clone(),
            value: c.text.clone(),
        })
        .collect();
    Some(EntityDetails {
        kind,
        name,
        topic: topic.map(str::to_string),
        status: d.text_of("Status").map(str::to_string),
        counts: counts(d),
        size_in_bytes: d.text_of("SizeInBytes").and_then(|t| t.parse().ok()),
        properties,
    })
}
