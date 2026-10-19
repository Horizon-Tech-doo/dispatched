//! Typed entity descriptions: every settable property of a queue, topic, subscription and rule,
//! and the entity's SAS policies, parsed from an ATOM entry. The parsed description element is
//! kept (`Description::raw`) so that an update writes back the elements this model does not know.

use super::rules::{CorrelationProperty, RuleFilter};
use super::xml::Node;
use crate::{Error, Result};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

/// .NET `TimeSpan` limits.
pub struct TimeSpan;

impl TimeSpan {
    /// `TimeSpan.MaxValue` (`P10675199DT2H48M5.4775807S`): "never" for TTL and auto-delete.
    pub const MAX: Duration = Duration::new(922_337_203_685, 477_580_700);
}

/// An entity's status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntityStatus {
    /// Sends and receives work.
    Active,
    /// Nothing works.
    Disabled,
    /// Receives work, sends are refused.
    SendDisabled,
    /// Sends work, receives are refused.
    ReceiveDisabled,
}

impl EntityStatus {
    /// The service's name: `Active`, `Disabled`, `SendDisabled`, `ReceiveDisabled`.
    pub fn as_str(self) -> &'static str {
        match self {
            EntityStatus::Active => "Active",
            EntityStatus::Disabled => "Disabled",
            EntityStatus::SendDisabled => "SendDisabled",
            EntityStatus::ReceiveDisabled => "ReceiveDisabled",
        }
    }
}

impl fmt::Display for EntityStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EntityStatus {
    type Err = Error;

    /// Accepts the service names in any case, and `send-disabled` style.
    fn from_str(s: &str) -> Result<Self> {
        let k: String = s
            .chars()
            .filter(|c| *c != '-' && *c != '_')
            .collect::<String>()
            .to_ascii_lowercase();
        match k.as_str() {
            "active" => Ok(EntityStatus::Active),
            "disabled" => Ok(EntityStatus::Disabled),
            "senddisabled" => Ok(EntityStatus::SendDisabled),
            "receivedisabled" => Ok(EntityStatus::ReceiveDisabled),
            _ => Err(Error::Invalid(format!(
                "`{s}` is not a status; use active, disabled, send-disabled or receive-disabled"
            ))),
        }
    }
}

/// A queue, topic or subscription, by name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum EntityRef {
    /// A queue.
    Queue {
        /// Queue name.
        name: String,
    },
    /// A topic.
    Topic {
        /// Topic name.
        name: String,
    },
    /// A topic subscription.
    Subscription {
        /// Its topic.
        topic: String,
        /// Subscription name.
        name: String,
    },
}

impl EntityRef {
    /// A queue.
    pub fn queue(name: &str) -> Self {
        EntityRef::Queue {
            name: name.to_string(),
        }
    }

    /// A topic.
    pub fn topic(name: &str) -> Self {
        EntityRef::Topic {
            name: name.to_string(),
        }
    }

    /// A subscription of `topic`.
    pub fn subscription(topic: &str, name: &str) -> Self {
        EntityRef::Subscription {
            topic: topic.to_string(),
            name: name.to_string(),
        }
    }

    /// An entity of `kind` at `path` (`topic/subscriptions/name` for a subscription).
    pub fn parse(kind: super::EntityKind, path: &str) -> Result<Self> {
        let p = crate::EntityPath::parse(path)?;
        match (kind, p.subscription) {
            (super::EntityKind::Queue, None) => Ok(Self::queue(&p.entity)),
            (super::EntityKind::Topic, None) => Ok(Self::topic(&p.entity)),
            (super::EntityKind::Subscription, Some(sub)) => Ok(Self::subscription(&p.entity, &sub)),
            (super::EntityKind::Subscription, None) => Err(Error::Invalid(format!(
                "`{path}` is not a subscription; use `topic/subscriptions/name`"
            ))),
            (k, Some(_)) => Err(Error::Invalid(format!(
                "`{path}` is a subscription, not a {}",
                k.as_str()
            ))),
        }
    }

    /// What it is.
    pub fn kind(&self) -> super::EntityKind {
        match self {
            EntityRef::Queue { .. } => super::EntityKind::Queue,
            EntityRef::Topic { .. } => super::EntityKind::Topic,
            EntityRef::Subscription { .. } => super::EntityKind::Subscription,
        }
    }

    /// The entity path: the queue or topic name, or `topic/Subscriptions/name`, as the
    /// confirmation of a production change names it.
    pub fn path(&self) -> String {
        self.entity_path().base()
    }

    /// The same entity as an [`crate::EntityPath`].
    pub fn entity_path(&self) -> crate::EntityPath {
        match self {
            EntityRef::Queue { name } | EntityRef::Topic { name } => crate::EntityPath::queue(name),
            EntityRef::Subscription { topic, name } => crate::EntityPath::subscription(topic, name),
        }
    }

    /// The same kind of entity, in the same place, named `name`.
    pub fn renamed(&self, name: &str) -> Self {
        match self {
            EntityRef::Queue { .. } => Self::queue(name),
            EntityRef::Topic { .. } => Self::topic(name),
            EntityRef::Subscription { topic, .. } => Self::subscription(topic, name),
        }
    }
}

impl fmt::Display for EntityRef {
    /// `queue orders`, `subscription events/Subscriptions/billing`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.kind().as_str(), self.path())
    }
}

/// Every settable property of a queue. `None` means "not set": on create the service default
/// applies, on update the property is left out (and the service resets it to its default).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct QueueSettings {
    /// Peek-lock duration.
    pub lock_duration: Option<Duration>,
    /// Maximum size in megabytes.
    pub max_size_in_megabytes: Option<u64>,
    /// Duplicate detection (set at creation only).
    pub requires_duplicate_detection: Option<bool>,
    /// Sessions (set at creation only).
    pub requires_session: Option<bool>,
    /// Default message time to live; [`TimeSpan::MAX`] is "never".
    pub default_message_time_to_live: Option<Duration>,
    /// Dead-letter messages that expire.
    pub dead_lettering_on_message_expiration: Option<bool>,
    /// Duplicate detection window.
    pub duplicate_detection_history_time_window: Option<Duration>,
    /// Deliveries before a message is dead-lettered.
    pub max_delivery_count: Option<u32>,
    /// Server-side batched operations.
    pub enable_batched_operations: Option<bool>,
    /// Status.
    pub status: Option<EntityStatus>,
    /// Auto-forward target.
    pub forward_to: Option<String>,
    /// Auto-forward target for dead-lettered messages.
    pub forward_dead_lettered_messages_to: Option<String>,
    /// Idle time after which the queue is deleted; [`TimeSpan::MAX`] is "never".
    pub auto_delete_on_idle: Option<Duration>,
    /// Partitioning (set at creation only).
    pub enable_partitioning: Option<bool>,
    /// Express entity.
    pub enable_express: Option<bool>,
    /// Maximum message size in kilobytes (Premium).
    pub max_message_size_in_kilobytes: Option<u64>,
    /// Free text.
    pub user_metadata: Option<String>,
}

/// Every settable property of a topic.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TopicSettings {
    /// Maximum size in megabytes.
    pub max_size_in_megabytes: Option<u64>,
    /// Duplicate detection (set at creation only).
    pub requires_duplicate_detection: Option<bool>,
    /// Duplicate detection window.
    pub duplicate_detection_history_time_window: Option<Duration>,
    /// Default message time to live.
    pub default_message_time_to_live: Option<Duration>,
    /// Server-side batched operations.
    pub enable_batched_operations: Option<bool>,
    /// Status.
    pub status: Option<EntityStatus>,
    /// Idle time after which the topic is deleted.
    pub auto_delete_on_idle: Option<Duration>,
    /// Partitioning (set at creation only).
    pub enable_partitioning: Option<bool>,
    /// Express entity.
    pub enable_express: Option<bool>,
    /// Ordered delivery.
    pub support_ordering: Option<bool>,
    /// Maximum message size in kilobytes (Premium).
    pub max_message_size_in_kilobytes: Option<u64>,
    /// Free text.
    pub user_metadata: Option<String>,
}

/// Every settable property of a subscription.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SubscriptionSettings {
    /// Peek-lock duration.
    pub lock_duration: Option<Duration>,
    /// Sessions (set at creation only).
    pub requires_session: Option<bool>,
    /// Default message time to live.
    pub default_message_time_to_live: Option<Duration>,
    /// Dead-letter messages that expire.
    pub dead_lettering_on_message_expiration: Option<bool>,
    /// Dead-letter messages whose filter evaluation fails.
    pub dead_lettering_on_filter_evaluation_exceptions: Option<bool>,
    /// Deliveries before a message is dead-lettered.
    pub max_delivery_count: Option<u32>,
    /// Server-side batched operations.
    pub enable_batched_operations: Option<bool>,
    /// Status.
    pub status: Option<EntityStatus>,
    /// Auto-forward target.
    pub forward_to: Option<String>,
    /// Auto-forward target for dead-lettered messages.
    pub forward_dead_lettered_messages_to: Option<String>,
    /// Idle time after which the subscription is deleted.
    pub auto_delete_on_idle: Option<Duration>,
    /// Free text.
    pub user_metadata: Option<String>,
}

/// A rule: its name, filter and optional SQL action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleSpec {
    /// Rule name.
    pub name: String,
    /// Its filter.
    pub filter: RuleFilter,
    /// SQL action, if any.
    pub action: Option<String>,
}

/// A right a SAS policy grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessRight {
    /// Receive.
    Listen,
    /// Send.
    Send,
    /// Manage the entity (needs Listen and Send as well).
    Manage,
}

impl AccessRight {
    /// The service's name.
    pub fn as_str(self) -> &'static str {
        match self {
            AccessRight::Listen => "Listen",
            AccessRight::Send => "Send",
            AccessRight::Manage => "Manage",
        }
    }
}

impl FromStr for AccessRight {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "listen" => Ok(AccessRight::Listen),
            "send" => Ok(AccessRight::Send),
            "manage" => Ok(AccessRight::Manage),
            _ => Err(Error::Invalid(format!(
                "`{s}` is not a right; use listen, send or manage"
            ))),
        }
    }
}

/// A shared access policy of an entity. Its keys never appear in `Debug` output.
#[derive(Clone)]
pub struct AccessPolicy {
    /// Policy (key) name.
    pub key_name: String,
    /// Granted rights.
    pub rights: Vec<AccessRight>,
    /// Primary key.
    pub primary_key: SecretString,
    /// Secondary key.
    pub secondary_key: SecretString,
}

impl fmt::Debug for AccessPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessPolicy")
            .field("key_name", &self.key_name)
            .field("rights", &self.rights)
            .field("primary_key", &"[redacted]")
            .field("secondary_key", &"[redacted]")
            .finish()
    }
}

impl PartialEq for AccessPolicy {
    fn eq(&self, other: &Self) -> bool {
        self.key_name == other.key_name
            && self.rights == other.rights
            && self.primary_key.expose_secret() == other.primary_key.expose_secret()
            && self.secondary_key.expose_secret() == other.secondary_key.expose_secret()
    }
}

impl Eq for AccessPolicy {}

/// An entity description: the typed settings, the SAS policies (queues and topics) and the
/// parsed description element, which keeps what the model does not know for an update.
/// The SAS keys are held only in `policies`; `raw` never holds them.
#[derive(Clone, PartialEq, Eq)]
pub struct Description<T> {
    /// Typed settings.
    pub settings: T,
    /// SAS policies (always empty for subscriptions and rules).
    pub policies: Vec<AccessPolicy>,
    /// The description element as read (empty for a new entity), without its policies.
    pub raw: Node,
}

impl<T: fmt::Debug> fmt::Debug for Description<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let raw: Vec<&str> = self.raw.children.iter().map(|c| c.name.as_str()).collect();
        f.debug_struct("Description")
            .field("settings", &self.settings)
            .field("policies", &self.policies)
            .field("raw", &raw)
            .finish()
    }
}

impl<T> Description<T> {
    /// A new entity with these settings: no policies, nothing read.
    pub fn new(settings: T) -> Self {
        Self {
            settings,
            policies: Vec::new(),
            raw: Node::default(),
        }
    }
}

impl<T: Default> Default for Description<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

// ---------------------------------------------------------------------------------------------
// Durations

/// An ISO-8601 duration as the service writes it (`PT1M`, `P14D`,
/// `P10675199DT2H48M5.4775807S`), at .NET's 100 ns resolution.
pub fn iso_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let ticks = d.subsec_nanos() / 100;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (h, m, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    let mut out = String::from("P");
    if days > 0 {
        out.push_str(&format!("{days}D"));
    }
    if h > 0 || m > 0 || s > 0 || ticks > 0 || days == 0 {
        out.push('T');
        if h > 0 {
            out.push_str(&format!("{h}H"));
        }
        if m > 0 {
            out.push_str(&format!("{m}M"));
        }
        if s > 0 || ticks > 0 || (h == 0 && m == 0) {
            out.push_str(&s.to_string());
            if ticks > 0 {
                let frac = format!("{ticks:07}");
                out.push('.');
                out.push_str(frac.trim_end_matches('0'));
            }
            out.push('S');
        }
    }
    out
}

/// Parse an ISO-8601 (XSD) duration. Years count 365 days and months 30, as .NET's
/// `XmlConvert` does; fractions of a second are kept to the nanosecond. Negative durations
/// are refused.
pub fn parse_iso_duration(s: &str) -> Result<Duration> {
    let bad = || {
        Error::Invalid(format!(
            "`{s}` is not an ISO-8601 duration (e.g. PT1M, P14D)"
        ))
    };
    let t = s.trim();
    let rest = t.strip_prefix('P').ok_or_else(bad)?;
    if rest.is_empty() {
        return Err(bad());
    }
    let mut nanos: u128 = 0;
    let mut in_time = false;
    let mut num = String::new();
    let mut any = false;
    for c in rest.chars() {
        match c {
            '0'..='9' | '.' => num.push(c),
            'T' if !in_time && num.is_empty() => in_time = true,
            unit => {
                if num.is_empty() {
                    return Err(bad());
                }
                let (int, frac) = match num.split_once('.') {
                    Some((i, f)) => (i, Some(f)),
                    None => (num.as_str(), None),
                };
                let n: u128 = if int.is_empty() {
                    0
                } else {
                    int.parse().map_err(|_| bad())?
                };
                let unit_nanos: u128 = match (in_time, unit) {
                    (false, 'Y') => 365 * 86_400 * 1_000_000_000,
                    (false, 'M') => 30 * 86_400 * 1_000_000_000,
                    (false, 'W') => 7 * 86_400 * 1_000_000_000,
                    (false, 'D') => 86_400 * 1_000_000_000,
                    (true, 'H') => 3_600 * 1_000_000_000,
                    (true, 'M') => 60 * 1_000_000_000,
                    (true, 'S') => 1_000_000_000,
                    _ => return Err(bad()),
                };
                nanos = n
                    .checked_mul(unit_nanos)
                    .and_then(|v| nanos.checked_add(v))
                    .ok_or_else(bad)?;
                if let Some(frac) = frac {
                    if unit != 'S'
                        || frac.contains('.')
                        || !frac.chars().all(|c| c.is_ascii_digit())
                    {
                        return Err(bad());
                    }
                    let digits: String = frac.chars().take(9).collect();
                    let scaled: u128 = format!("{digits:0<9}").parse().map_err(|_| bad())?;
                    nanos += scaled;
                }
                num.clear();
                any = true;
            }
        }
    }
    if !num.is_empty() || !any {
        return Err(bad());
    }
    let secs = u64::try_from(nanos / 1_000_000_000).map_err(|_| bad())?;
    Ok(Duration::new(secs, (nanos % 1_000_000_000) as u32))
}

// ---------------------------------------------------------------------------------------------
// Parsing

/// Elements the service reports but never accepts back; an update leaves them out.
pub(crate) const READ_ONLY: &[&str] = &[
    "AccessedAt",
    "CreatedAt",
    "UpdatedAt",
    "MessageCount",
    "SizeInBytes",
    "CountDetails",
    "SubscriptionCount",
    "EntityAvailabilityStatus",
    "SkippedUpdate",
    "DefaultRuleDescription",
];

fn description_of<'a>(entry: &'a Node, kind: &str) -> Result<&'a Node> {
    if entry.name == kind {
        return Ok(entry);
    }
    entry
        .path(&["content", kind])
        .ok_or_else(|| Error::Protocol(format!("the entry holds no {kind}")))
}

fn text(d: &Node, name: &str) -> Option<String> {
    d.child(name)
        .map(|n| n.text.clone())
        .filter(|t| !t.is_empty())
}

fn dur(d: &Node, name: &str) -> Result<Option<Duration>> {
    text(d, name)
        .map(|t| {
            parse_iso_duration(&t)
                .map_err(|_| Error::Protocol(format!("{name}: `{t}` is not a duration")))
        })
        .transpose()
}

fn flag(d: &Node, name: &str) -> Result<Option<bool>> {
    text(d, name)
        .map(|t| match t.to_ascii_lowercase().as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(Error::Protocol(format!("{name}: `{t}` is not a boolean"))),
        })
        .transpose()
}

fn num<N: FromStr>(d: &Node, name: &str) -> Result<Option<N>> {
    text(d, name)
        .map(|t| {
            t.parse()
                .map_err(|_| Error::Protocol(format!("{name}: `{t}` is not a number")))
        })
        .transpose()
}

fn status(d: &Node) -> Result<Option<EntityStatus>> {
    text(d, "Status")
        .map(|t| {
            t.parse()
                .map_err(|_| Error::Protocol(format!("Status: `{t}` is not a settable status")))
        })
        .transpose()
}

fn policies(d: &Node) -> Result<Vec<AccessPolicy>> {
    let Some(rules) = d.child("AuthorizationRules") else {
        return Ok(Vec::new());
    };
    rules
        .children_named("AuthorizationRule")
        .map(|r| {
            let key_name = text(r, "KeyName")
                .ok_or_else(|| Error::Protocol("an authorization rule has no KeyName".into()))?;
            let rights = r
                .child("Rights")
                .map(|rs| {
                    rs.children
                        .iter()
                        .map(|x| {
                            x.text.parse().map_err(|_| {
                                Error::Protocol(format!(
                                    "policy {key_name}: unknown right `{}`",
                                    x.text
                                ))
                            })
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default();
            Ok(AccessPolicy {
                rights,
                primary_key: SecretString::from(text(r, "PrimaryKey").unwrap_or_default()),
                secondary_key: SecretString::from(text(r, "SecondaryKey").unwrap_or_default()),
                key_name,
            })
        })
        .collect()
}

/// The description element without its policies, so no key is kept outside `policies`.
fn raw_without_keys(d: &Node) -> Node {
    let mut raw = d.clone();
    raw.children.retain(|c| c.name != "AuthorizationRules");
    raw
}

/// A queue's description from its ATOM entry (or the `QueueDescription` element itself).
pub fn parse_queue(entry: &Node) -> Result<Description<QueueSettings>> {
    let d = description_of(entry, "QueueDescription")?;
    Ok(Description {
        settings: QueueSettings {
            lock_duration: dur(d, "LockDuration")?,
            max_size_in_megabytes: num(d, "MaxSizeInMegabytes")?,
            requires_duplicate_detection: flag(d, "RequiresDuplicateDetection")?,
            requires_session: flag(d, "RequiresSession")?,
            default_message_time_to_live: dur(d, "DefaultMessageTimeToLive")?,
            dead_lettering_on_message_expiration: flag(d, "DeadLetteringOnMessageExpiration")?,
            duplicate_detection_history_time_window: dur(d, "DuplicateDetectionHistoryTimeWindow")?,
            max_delivery_count: num(d, "MaxDeliveryCount")?,
            enable_batched_operations: flag(d, "EnableBatchedOperations")?,
            status: status(d)?,
            forward_to: text(d, "ForwardTo"),
            forward_dead_lettered_messages_to: text(d, "ForwardDeadLetteredMessagesTo"),
            auto_delete_on_idle: dur(d, "AutoDeleteOnIdle")?,
            enable_partitioning: flag(d, "EnablePartitioning")?,
            enable_express: flag(d, "EnableExpress")?,
            max_message_size_in_kilobytes: num(d, "MaxMessageSizeInKilobytes")?,
            user_metadata: text(d, "UserMetadata"),
        },
        policies: policies(d)?,
        raw: raw_without_keys(d),
    })
}

/// A topic's description from its ATOM entry.
pub fn parse_topic(entry: &Node) -> Result<Description<TopicSettings>> {
    let d = description_of(entry, "TopicDescription")?;
    Ok(Description {
        settings: TopicSettings {
            max_size_in_megabytes: num(d, "MaxSizeInMegabytes")?,
            requires_duplicate_detection: flag(d, "RequiresDuplicateDetection")?,
            duplicate_detection_history_time_window: dur(d, "DuplicateDetectionHistoryTimeWindow")?,
            default_message_time_to_live: dur(d, "DefaultMessageTimeToLive")?,
            enable_batched_operations: flag(d, "EnableBatchedOperations")?,
            status: status(d)?,
            auto_delete_on_idle: dur(d, "AutoDeleteOnIdle")?,
            enable_partitioning: flag(d, "EnablePartitioning")?,
            enable_express: flag(d, "EnableExpress")?,
            support_ordering: flag(d, "SupportOrdering")?,
            max_message_size_in_kilobytes: num(d, "MaxMessageSizeInKilobytes")?,
            user_metadata: text(d, "UserMetadata"),
        },
        policies: policies(d)?,
        raw: raw_without_keys(d),
    })
}

/// A subscription's description from its ATOM entry.
pub fn parse_subscription(entry: &Node) -> Result<Description<SubscriptionSettings>> {
    let d = description_of(entry, "SubscriptionDescription")?;
    Ok(Description {
        settings: SubscriptionSettings {
            lock_duration: dur(d, "LockDuration")?,
            requires_session: flag(d, "RequiresSession")?,
            default_message_time_to_live: dur(d, "DefaultMessageTimeToLive")?,
            dead_lettering_on_message_expiration: flag(d, "DeadLetteringOnMessageExpiration")?,
            dead_lettering_on_filter_evaluation_exceptions: flag(
                d,
                "DeadLetteringOnFilterEvaluationExceptions",
            )?,
            max_delivery_count: num(d, "MaxDeliveryCount")?,
            enable_batched_operations: flag(d, "EnableBatchedOperations")?,
            status: status(d)?,
            forward_to: text(d, "ForwardTo"),
            forward_dead_lettered_messages_to: text(d, "ForwardDeadLetteredMessagesTo"),
            auto_delete_on_idle: dur(d, "AutoDeleteOnIdle")?,
            user_metadata: text(d, "UserMetadata"),
        },
        policies: Vec::new(),
        raw: raw_without_keys(d),
    })
}

/// A filter element by its `i:type`: `SqlFilter`, `TrueFilter`, `FalseFilter` (as SQL `1=0`)
/// and `CorrelationFilter`; anything else is `Unknown`.
pub(crate) fn filter_from(f: &Node) -> RuleFilter {
    let sql = || f.text_of("SqlExpression").map(str::to_string);
    match f.xsi_type() {
        Some("TrueFilter") => RuleFilter::True,
        Some("FalseFilter") => RuleFilter::Sql {
            expression: sql().unwrap_or_else(|| "1=0".into()),
        },
        Some("SqlFilter") => match sql() {
            Some(expression) => RuleFilter::Sql { expression },
            None => RuleFilter::Unknown,
        },
        Some("CorrelationFilter") => correlation_from(f),
        _ => RuleFilter::Unknown,
    }
}

pub(crate) fn correlation_from(f: &Node) -> RuleFilter {
    let properties = f
        .child("Properties")
        .map(|p| {
            p.children_named("KeyValueOfstringanyType")
                .filter_map(|kv| {
                    Some(CorrelationProperty {
                        name: kv.child("Key")?.text.clone(),
                        value: kv
                            .child("Value")
                            .map(|v| v.text.clone())
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    RuleFilter::Correlation {
        correlation_id: text(f, "CorrelationId"),
        message_id: text(f, "MessageId"),
        to: text(f, "To"),
        reply_to: text(f, "ReplyTo"),
        subject: text(f, "Label"),
        session_id: text(f, "SessionId"),
        reply_to_session_id: text(f, "ReplyToSessionId"),
        content_type: text(f, "ContentType"),
        properties,
    }
}

/// A rule's description from its ATOM entry. The name comes from `Name`, else the entry title.
pub fn parse_rule(entry: &Node) -> Result<Description<RuleSpec>> {
    let d = description_of(entry, "RuleDescription")?;
    let name = text(d, "Name")
        .or_else(|| entry.text_of("title").map(str::to_string))
        .ok_or_else(|| Error::Protocol("the rule has no name".into()))?;
    let filter = d
        .child("Filter")
        .map(filter_from)
        .unwrap_or(RuleFilter::Unknown);
    let action = d
        .child("Action")
        .filter(|a| a.xsi_type() != Some("EmptyRuleAction"))
        .and_then(|a| a.text_of("SqlExpression"))
        .map(str::to_string);
    Ok(Description {
        settings: RuleSpec {
            name,
            filter,
            action,
        },
        policies: Vec::new(),
        raw: d.clone(),
    })
}

/// Refuse a filter the service would accept but that does not do what it looks like, or that
/// cannot be written. A correlation filter with no field set and no named property lets no
/// message through (#794), so it is refused.
pub fn validate_filter(f: &RuleFilter) -> Result<()> {
    match f {
        RuleFilter::True => Ok(()),
        RuleFilter::Sql { expression } if expression.trim().is_empty() => Err(Error::Invalid(
            "a SQL filter needs an expression; use `1=1` to match every message".into(),
        )),
        RuleFilter::Sql { .. } => Ok(()),
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
            let set = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.trim().is_empty());
            let any_field = [
                correlation_id,
                message_id,
                to,
                reply_to,
                subject,
                session_id,
                reply_to_session_id,
                content_type,
            ]
            .into_iter()
            .any(set);
            let any_property = properties.iter().any(|p| !p.name.trim().is_empty());
            if any_field || any_property {
                Ok(())
            } else {
                Err(Error::Invalid(
                    "this correlation filter has no field and no named property set, so it \
                     matches no message (#794); set a field or a property, or, to match every \
                     message, use a SQL filter `1=1` or keep the default rule"
                        .into(),
                ))
            }
        }
        RuleFilter::Unknown => Err(Error::Invalid(
            "the filter is of a type Dispatched cannot write; use a SQL or correlation filter"
                .into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mgmt::xml;

    #[test]
    fn iso_durations_round_trip_including_max() {
        for (text, d) in [
            ("PT1M", Duration::from_secs(60)),
            ("P14D", Duration::from_secs(14 * 86_400)),
            ("PT10M", Duration::from_secs(600)),
            ("PT0S", Duration::ZERO),
            ("PT30S", Duration::from_secs(30)),
            ("P1DT2H3M4.5S", Duration::new(86_400 + 7_384, 500_000_000)),
            ("P10675199DT2H48M5.4775807S", TimeSpan::MAX),
        ] {
            assert_eq!(iso_duration(d), text, "{d:?}");
            assert_eq!(parse_iso_duration(text).unwrap(), d, "{text}");
        }
        assert_eq!(
            parse_iso_duration("P1Y").unwrap(),
            Duration::from_secs(365 * 86_400)
        );
        assert_eq!(
            parse_iso_duration("PT0.0000001S").unwrap(),
            Duration::from_nanos(100)
        );
        for bad in ["", "P", "PT", "1M", "-PT1M", "PT1.5M", "PTM", "P1H", "PT1X"] {
            assert!(parse_iso_duration(bad).is_err(), "{bad}");
        }
    }

    fn correlation(props: &[(&str, &str)]) -> RuleFilter {
        RuleFilter::Correlation {
            correlation_id: None,
            message_id: None,
            to: None,
            reply_to: None,
            subject: Some(" ".into()),
            session_id: None,
            reply_to_session_id: None,
            content_type: None,
            properties: props
                .iter()
                .map(|(n, v)| CorrelationProperty {
                    name: n.to_string(),
                    value: v.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn empty_correlation_filter_is_refused() {
        let e = validate_filter(&correlation(&[])).unwrap_err();
        let m = e.to_string();
        assert!(m.contains("matches no message"), "{m}");
        assert!(m.contains("match every"), "{m}");
        assert!(m.contains("1=1") && m.contains("default rule"), "{m}");
        assert!(
            validate_filter(&correlation(&[("", "x"), ("  ", "y")])).is_err(),
            "properties with empty names do not count"
        );
        assert!(validate_filter(&correlation(&[("tenant", "")])).is_ok());
        assert!(validate_filter(&RuleFilter::True).is_ok());
        assert!(
            validate_filter(&RuleFilter::Sql {
                expression: " ".into()
            })
            .is_err()
        );
        assert!(validate_filter(&RuleFilter::Unknown).is_err());
    }

    #[test]
    fn keys_never_appear_in_debug_output() {
        let policy = AccessPolicy {
            key_name: "app".into(),
            rights: vec![AccessRight::Listen, AccessRight::Send],
            primary_key: SecretString::from("PRIMARY-SECRET-1"),
            secondary_key: SecretString::from("SECONDARY-SECRET-2"),
        };
        let text = format!("{policy:?}");
        assert!(text.contains("app"));
        assert!(!text.contains("PRIMARY-SECRET-1") && !text.contains("SECONDARY-SECRET-2"));
        let entry =
            xml::parse_exact(include_str!("../../tests/fixtures/queue_entry_full.xml")).unwrap();
        let d = parse_queue(&entry).unwrap();
        let text = format!("{d:?}");
        assert!(!d.policies.is_empty());
        for secret in ["primary-key-value", "secondary-key-value"] {
            assert!(!text.contains(secret), "{secret} in {text}");
            assert!(
                !format!("{:?}", d.raw).contains(secret),
                "the raw element keeps no key"
            );
        }
    }

    #[test]
    fn statuses_parse_in_any_spelling() {
        assert_eq!(
            "send-disabled".parse::<EntityStatus>().unwrap(),
            EntityStatus::SendDisabled
        );
        assert_eq!(
            "ReceiveDisabled".parse::<EntityStatus>().unwrap(),
            EntityStatus::ReceiveDisabled
        );
        assert!("Creating".parse::<EntityStatus>().is_err());
    }
}
