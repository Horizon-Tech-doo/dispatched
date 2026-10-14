//! Entity models parsed from ATOM entries.

use super::xml::Node;
use serde::Serialize;
use std::str::FromStr;

/// Message counts from `CountDetails`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EntityCounts {
    /// Active messages.
    pub active: u64,
    /// Dead-lettered messages.
    pub dead_letter: u64,
    /// Scheduled messages.
    pub scheduled: u64,
    /// Messages pending transfer.
    pub transfer: u64,
    /// Transfer dead-letter messages.
    pub transfer_dead_letter: u64,
    /// Counted by peeking (the service did not report counts); may be capped.
    #[serde(default)]
    pub approximate: bool,
    /// A peek count hit its cap, so the numbers are lower bounds. Implies `approximate`.
    #[serde(default)]
    pub truncated: bool,
}

/// A queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueueInfo {
    /// Queue name.
    pub name: String,
    /// `Active`, `Disabled`, `SendDisabled`, `ReceiveDisabled`.
    pub status: Option<String>,
    /// Counts; `None` if the service did not report them.
    pub counts: Option<EntityCounts>,
    /// Size in bytes.
    pub size_in_bytes: Option<u64>,
    /// Session-enabled.
    pub requires_session: bool,
    /// Max delivery count.
    pub max_delivery_count: Option<u32>,
    /// ISO-8601 lock duration (e.g. `PT1M`).
    pub lock_duration: Option<String>,
    /// Auto-forward target.
    pub forward_to: Option<String>,
}

/// A topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TopicInfo {
    /// Topic name.
    pub name: String,
    /// Status.
    pub status: Option<String>,
    /// Size in bytes.
    pub size_in_bytes: Option<u64>,
    /// Number of subscriptions.
    pub subscription_count: Option<u32>,
}

/// A topic subscription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubscriptionInfo {
    /// Parent topic.
    pub topic: String,
    /// Subscription name.
    pub name: String,
    /// Status.
    pub status: Option<String>,
    /// Counts; `None` if not reported.
    pub counts: Option<EntityCounts>,
    /// Session-enabled.
    pub requires_session: bool,
    /// Max delivery count.
    pub max_delivery_count: Option<u32>,
    /// Auto-forward target.
    pub forward_to: Option<String>,
}

/// Entries of a feed, or the root itself when it is a single entry.
pub(crate) fn entries(root: &Node) -> Vec<&Node> {
    match root.name.as_str() {
        "feed" => root.children_named("entry").collect(),
        "entry" => vec![root],
        _ => Vec::new(),
    }
}

fn description<'a>(entry: &'a Node, kind: &str) -> Option<&'a Node> {
    entry.path(&["content", kind])
}

fn opt<T: FromStr>(node: &Node, name: &str) -> Option<T> {
    node.text_of(name).and_then(|t| t.parse().ok())
}

fn flag(node: &Node, name: &str) -> bool {
    node.text_of(name)
        .is_some_and(|t| t.eq_ignore_ascii_case("true"))
}

fn text(node: &Node, name: &str) -> Option<String> {
    node.text_of(name).map(str::to_string)
}

fn counts(desc: &Node) -> Option<EntityCounts> {
    let c = desc.child("CountDetails")?;
    Some(EntityCounts {
        active: opt(c, "ActiveMessageCount").unwrap_or_default(),
        dead_letter: opt(c, "DeadLetterMessageCount").unwrap_or_default(),
        scheduled: opt(c, "ScheduledMessageCount").unwrap_or_default(),
        transfer: opt(c, "TransferMessageCount").unwrap_or_default(),
        transfer_dead_letter: opt(c, "TransferDeadLetterMessageCount").unwrap_or_default(),
        approximate: false,
        truncated: false,
    })
}

pub(crate) fn queue_from_entry(entry: &Node) -> Option<QueueInfo> {
    let name = entry.text_of("title")?.to_string();
    let d = description(entry, "QueueDescription")?;
    Some(QueueInfo {
        name,
        status: text(d, "Status"),
        counts: counts(d),
        size_in_bytes: opt(d, "SizeInBytes"),
        requires_session: flag(d, "RequiresSession"),
        max_delivery_count: opt(d, "MaxDeliveryCount"),
        lock_duration: text(d, "LockDuration"),
        forward_to: text(d, "ForwardTo"),
    })
}

pub(crate) fn topic_from_entry(entry: &Node) -> Option<TopicInfo> {
    let name = entry.text_of("title")?.to_string();
    let d = description(entry, "TopicDescription")?;
    Some(TopicInfo {
        name,
        status: text(d, "Status"),
        size_in_bytes: opt(d, "SizeInBytes"),
        subscription_count: opt(d, "SubscriptionCount"),
    })
}

pub(crate) fn subscription_from_entry(topic: &str, entry: &Node) -> Option<SubscriptionInfo> {
    let name = entry.text_of("title")?.to_string();
    let d = description(entry, "SubscriptionDescription")?;
    Some(SubscriptionInfo {
        topic: topic.to_string(),
        name,
        status: text(d, "Status"),
        counts: counts(d),
        requires_session: flag(d, "RequiresSession"),
        max_delivery_count: opt(d, "MaxDeliveryCount"),
        forward_to: text(d, "ForwardTo"),
    })
}
