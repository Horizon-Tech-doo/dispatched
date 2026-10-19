//! The incumbent's (ServiceBusExplorer's) entity export format: an `Entities` document in the
//! `http://schemas.microsoft.com/servicebusexplorer` namespace, with .NET `TimeSpan` text,
//! `True`/`False`, bare `ForwardTo` names and correlation properties as `key: value`.

use super::model::{
    AccessPolicy, AccessRight, Description, EntityRef, EntityStatus, QueueSettings, RuleSpec,
    SubscriptionSettings, TopicSettings, parse_iso_duration,
};
use super::rules::{CorrelationProperty, RuleFilter};
use super::xml::{self, Node};
use crate::explorer::Explorer;
use crate::{Error, Result};
use quick_xml::escape::{escape, partial_escape};
use secrecy::{ExposeSecret, SecretString};
use std::time::Duration;

/// The incumbent's XML namespace.
pub const NAMESPACE: &str = "http://schemas.microsoft.com/servicebusexplorer";

/// A subscription with its rules, for an export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSubscription {
    /// Subscription name.
    pub name: String,
    /// Its settings.
    pub d: Description<SubscriptionSettings>,
    /// Its rules.
    pub rules: Vec<RuleSpec>,
}

/// A queue, or a topic with its subscriptions, for an export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportEntity {
    /// A queue with its policies.
    Queue {
        /// Queue name.
        name: String,
        /// Its description.
        d: Description<QueueSettings>,
    },
    /// A topic with its policies and subscriptions.
    Topic {
        /// Topic name.
        name: String,
        /// Its description.
        d: Description<TopicSettings>,
        /// Its subscriptions (all, or the selected ones).
        subscriptions: Vec<ExportSubscription>,
    },
}

/// A SAS policy as an incumbent file holds it. A key the file leaves out (the default export
/// omits keys) is `None`, never a made-up key: an import with `--overwrite` keeps the existing
/// key then, and only a policy new to the entity gets fresh ones.
#[derive(Clone)]
pub struct ImportPolicy {
    /// Policy (key) name.
    pub key_name: String,
    /// Granted rights.
    pub rights: Vec<AccessRight>,
    /// Primary key, when the file has it.
    pub primary_key: Option<SecretString>,
    /// Secondary key, when the file has it.
    pub secondary_key: Option<SecretString>,
}

impl ImportPolicy {
    /// Whether the file holds a key of this policy.
    pub fn has_keys(&self) -> bool {
        self.primary_key.is_some() || self.secondary_key.is_some()
    }

    /// The policy written onto an entity: the file's rights; each key from the file when it
    /// has one, else the `existing` policy's (same name), else a new one.
    pub fn applied(&self, existing: Option<&AccessPolicy>) -> AccessPolicy {
        let key = |file: &Option<SecretString>, current: Option<&SecretString>| {
            file.clone()
                .or_else(|| current.cloned())
                .unwrap_or_else(super::policies::new_key)
        };
        AccessPolicy {
            key_name: self.key_name.clone(),
            rights: self.rights.clone(),
            primary_key: key(&self.primary_key, existing.map(|e| &e.primary_key)),
            secondary_key: key(&self.secondary_key, existing.map(|e| &e.secondary_key)),
        }
    }
}

impl std::fmt::Debug for ImportPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let shown = |k: &Option<SecretString>| k.as_ref().map(|_| "[REDACTED]");
        f.debug_struct("ImportPolicy")
            .field("key_name", &self.key_name)
            .field("rights", &self.rights)
            .field("primary_key", &shown(&self.primary_key))
            .field("secondary_key", &shown(&self.secondary_key))
            .finish()
    }
}

impl PartialEq for ImportPolicy {
    fn eq(&self, other: &Self) -> bool {
        let same = |a: &Option<SecretString>, b: &Option<SecretString>| {
            a.as_ref().map(|k| k.expose_secret()) == b.as_ref().map(|k| k.expose_secret())
        };
        self.key_name == other.key_name
            && self.rights == other.rights
            && same(&self.primary_key, &other.primary_key)
            && same(&self.secondary_key, &other.secondary_key)
    }
}

impl Eq for ImportPolicy {}

/// An entity read from an incumbent file. Subscriptions carry their rules; an import creates
/// queues, then topics, then subscriptions, then their other rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportEntity {
    /// A queue.
    Queue {
        /// Queue name.
        name: String,
        /// Its settings (`d.policies` is empty: the file's policies are `policies`).
        d: Description<QueueSettings>,
        /// Its policies, with the keys the file holds.
        policies: Vec<ImportPolicy>,
    },
    /// A topic.
    Topic {
        /// Topic name.
        name: String,
        /// Its settings (`d.policies` is empty: the file's policies are `policies`).
        d: Description<TopicSettings>,
        /// Its policies, with the keys the file holds.
        policies: Vec<ImportPolicy>,
    },
    /// A subscription with its rules.
    Subscription {
        /// Its topic (`TopicPath`, or the enclosing topic).
        topic: String,
        /// Subscription name.
        name: String,
        /// Its settings.
        d: Description<SubscriptionSettings>,
        /// Its rules, in file order.
        rules: Vec<RuleSpec>,
    },
}

impl ImportEntity {
    /// The entity it creates.
    pub fn entity(&self) -> EntityRef {
        match self {
            ImportEntity::Queue { name, .. } => EntityRef::queue(name),
            ImportEntity::Topic { name, .. } => EntityRef::topic(name),
            ImportEntity::Subscription { topic, name, .. } => EntityRef::subscription(topic, name),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// TimeSpans

/// A duration in .NET's `TimeSpan` "c" format: `00:01:00`, `14.00:00:00`,
/// `10675199.02:48:05.4775807`.
pub fn dotnet_timespan(d: Duration) -> String {
    let secs = d.as_secs();
    let ticks = d.subsec_nanos() / 100;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let mut s = String::new();
    if days > 0 {
        s.push_str(&format!("{days}."));
    }
    s.push_str(&format!(
        "{:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    ));
    if ticks > 0 {
        s.push_str(&format!(".{ticks:07}"));
    }
    s
}

/// Parse a .NET `TimeSpan` (`[d.]hh:mm:ss[.fffffff]`, `d:hh:mm:ss`, or whole days `d`), or an
/// ISO-8601 duration. Negative spans are refused.
pub fn parse_dotnet_timespan(s: &str) -> Result<Duration> {
    let t = s.trim();
    if t.starts_with('P') {
        return parse_iso_duration(t);
    }
    let bad = || {
        Error::Invalid(format!(
            "`{s}` is not a time span (e.g. 00:01:00, 14.00:00:00)"
        ))
    };
    let num = |p: &str| -> Result<u64> {
        if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
            return Err(bad());
        }
        p.parse().map_err(|_| bad())
    };
    let parts: Vec<&str> = t.split(':').collect();
    let (days, h, m, sec) = match parts.as_slice() {
        [d] => (num(d)?, 0, 0, "0"),
        [dh, m, s] => {
            let (d, h) = match dh.split_once('.') {
                Some((d, h)) => (num(d)?, num(h)?),
                None => (0, num(dh)?),
            };
            (d, h, num(m)?, *s)
        }
        [d, h, m, s] => (num(d)?, num(h)?, num(m)?, *s),
        _ => return Err(bad()),
    };
    let (whole, frac) = match sec.split_once('.') {
        Some((w, f)) => (num(w)?, Some(f)),
        None => (num(sec)?, None),
    };
    if h > 23 || m > 59 || whole > 59 {
        return Err(bad());
    }
    let nanos = match frac {
        None => 0,
        Some(f) => {
            if f.is_empty() || f.len() > 9 || !f.chars().all(|c| c.is_ascii_digit()) {
                return Err(bad());
            }
            format!("{f:0<9}").parse::<u32>().map_err(|_| bad())?
        }
    };
    let secs = days
        .checked_mul(86_400)
        .and_then(|v| v.checked_add(h * 3600 + m * 60 + whole))
        .ok_or_else(bad)?;
    Ok(Duration::new(secs, nanos))
}

// ---------------------------------------------------------------------------------------------
// Export

/// An indented XML writer for the incumbent's layout.
struct Out {
    s: String,
    depth: usize,
}

impl Out {
    fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.s.push_str("  ");
        }
        self.s.push_str(text);
        self.s.push('\n');
    }
    fn open(&mut self, name: &str) {
        self.line(&format!("<{name}>"));
        self.depth += 1;
    }
    fn close(&mut self, name: &str) {
        self.depth -= 1;
        self.line(&format!("</{name}>"));
    }
    fn leaf(&mut self, name: &str, value: &str) {
        self.line(&leaf(name, value));
    }
    fn opt<T>(&mut self, name: &str, v: &Option<T>, f: impl Fn(&T) -> String) {
        if let Some(v) = v {
            self.leaf(name, &f(v));
        }
    }
    fn flag(&mut self, name: &str, v: Option<bool>) {
        self.opt(name, &v, |b| if *b { "True" } else { "False" }.to_string());
    }
    fn span(&mut self, name: &str, v: Option<Duration>) {
        self.opt(name, &v, |d| dotnet_timespan(*d));
    }
    fn num<N: ToString>(&mut self, name: &str, v: Option<N>) {
        self.opt(name, &v, |n| n.to_string());
    }
    fn text(&mut self, name: &str, v: &Option<String>) {
        self.opt(name, v, |s| s.clone());
    }
    fn status(&mut self, v: Option<EntityStatus>) {
        self.opt("Status", &v, |s| s.as_str().to_string());
    }
}

fn leaf(name: &str, value: &str) -> String {
    if value.is_empty() {
        format!("<{name} />")
    } else {
        format!("<{name}>{}</{name}>", partial_escape(value))
    }
}

/// The entity name of a forwarding target: `sb://ns/archive` becomes `archive`.
fn bare(target: &Option<String>) -> Option<String> {
    target.as_ref().map(|t| match url::Url::parse(t) {
        Ok(u) if t.contains("://") => {
            let path = u.path().trim_start_matches('/');
            percent_encoding::percent_decode_str(path)
                .decode_utf8_lossy()
                .into_owned()
        }
        _ => t.clone(),
    })
}

fn write_policies(o: &mut Out, policies: &[AccessPolicy], include_keys: bool) {
    if policies.is_empty() {
        return;
    }
    o.open("AuthorizationRules");
    for p in policies {
        o.open("SharedAccessAuthorizationRule");
        o.leaf("ClaimType", "SharedAccessKey");
        o.leaf("ClaimValue", "None");
        let rights: String = p.rights.iter().map(|r| leaf("Right", r.as_str())).collect();
        o.line(&format!("<Rights>{rights}</Rights>"));
        o.leaf("KeyName", &p.key_name);
        if include_keys {
            o.leaf("PrimaryKey", p.primary_key.expose_secret());
            o.leaf("SecondaryKey", p.secondary_key.expose_secret());
        }
        o.close("SharedAccessAuthorizationRule");
    }
    o.close("AuthorizationRules");
}

fn write_rule(o: &mut Out, r: &RuleSpec) {
    o.open("Rule");
    o.leaf("Name", &r.name);
    match &r.filter {
        RuleFilter::True | RuleFilter::Unknown => o.line("<Filter />"),
        RuleFilter::Sql { expression } => {
            o.open("Filter");
            o.line(&format!(
                "<SqlFilter>{}</SqlFilter>",
                leaf("SqlExpression", expression)
            ));
            o.close("Filter");
        }
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
            o.open("Filter");
            o.open("CorrelationFilter");
            for (name, v) in [
                ("CorrelationId", correlation_id),
                ("MessageId", message_id),
                ("To", to),
                ("ReplyTo", reply_to),
                ("Label", subject),
                ("SessionId", session_id),
                ("ReplyToSessionId", reply_to_session_id),
                ("ContentType", content_type),
            ] {
                o.leaf(name, v.as_deref().unwrap_or(""));
            }
            if properties.is_empty() {
                o.line("<Properties />");
            } else {
                o.open("Properties");
                for p in properties {
                    o.leaf("Property", &format!("{}: {}", p.name, p.value));
                }
                o.close("Properties");
            }
            o.close("CorrelationFilter");
            o.close("Filter");
        }
    }
    match &r.action {
        Some(a) => {
            o.open("Action");
            o.line(&format!(
                "<SqlRuleAction>{}</SqlRuleAction>",
                leaf("SqlExpression", a)
            ));
            o.close("Action");
        }
        None => o.line("<Action />"),
    }
    o.close("Rule");
}

/// The incumbent's `Entities` document for `entities` of namespace `ns_name`. Keys are
/// written only with `include_keys` (the file then holds secrets).
pub fn export_entities(ns_name: &str, entities: &[ExportEntity], include_keys: bool) -> String {
    let mut o = Out {
        s: String::new(),
        depth: 0,
    };
    o.line(&format!(
        "<Entities xmlns=\"{NAMESPACE}\" serviceBusNamespace=\"{}\">",
        escape(ns_name)
    ));
    o.depth += 1;
    let queues: Vec<_> = entities
        .iter()
        .filter_map(|e| match e {
            ExportEntity::Queue { name, d } => Some((name, d)),
            _ => None,
        })
        .collect();
    if !queues.is_empty() {
        o.open("Queues");
        for (name, d) in queues {
            let s = &d.settings;
            o.open("Queue");
            o.leaf("Path", name);
            o.span("LockDuration", s.lock_duration);
            o.num("MaxSizeInMegabytes", s.max_size_in_megabytes);
            o.flag("RequiresDuplicateDetection", s.requires_duplicate_detection);
            o.flag("RequiresSession", s.requires_session);
            o.span("DefaultMessageTimeToLive", s.default_message_time_to_live);
            o.flag(
                "EnableDeadLetteringOnMessageExpiration",
                s.dead_lettering_on_message_expiration,
            );
            o.span(
                "DuplicateDetectionHistoryTimeWindow",
                s.duplicate_detection_history_time_window,
            );
            o.num("MaxDeliveryCount", s.max_delivery_count);
            o.flag("EnableBatchedOperations", s.enable_batched_operations);
            o.status(s.status);
            o.text("ForwardTo", &bare(&s.forward_to));
            o.text(
                "ForwardDeadLetteredMessagesTo",
                &bare(&s.forward_dead_lettered_messages_to),
            );
            o.span("AutoDeleteOnIdle", s.auto_delete_on_idle);
            o.flag("EnablePartitioning", s.enable_partitioning);
            o.flag("EnableExpress", s.enable_express);
            o.text("UserMetadata", &s.user_metadata);
            o.num("MaxMessageSizeInKilobytes", s.max_message_size_in_kilobytes);
            write_policies(&mut o, &d.policies, include_keys);
            o.close("Queue");
        }
        o.close("Queues");
    }
    let topics: Vec<_> = entities
        .iter()
        .filter_map(|e| match e {
            ExportEntity::Topic {
                name,
                d,
                subscriptions,
            } => Some((name, d, subscriptions)),
            _ => None,
        })
        .collect();
    if !topics.is_empty() {
        o.open("Topics");
        for (name, d, subscriptions) in topics {
            let s = &d.settings;
            o.open("Topic");
            o.leaf("Path", name);
            o.num("MaxSizeInMegabytes", s.max_size_in_megabytes);
            o.flag("RequiresDuplicateDetection", s.requires_duplicate_detection);
            o.span("DefaultMessageTimeToLive", s.default_message_time_to_live);
            o.span(
                "DuplicateDetectionHistoryTimeWindow",
                s.duplicate_detection_history_time_window,
            );
            o.flag("EnableBatchedOperations", s.enable_batched_operations);
            o.status(s.status);
            o.span("AutoDeleteOnIdle", s.auto_delete_on_idle);
            o.flag("EnablePartitioning", s.enable_partitioning);
            o.flag("EnableExpress", s.enable_express);
            o.flag("SupportOrdering", s.support_ordering);
            o.text("UserMetadata", &s.user_metadata);
            o.num("MaxMessageSizeInKilobytes", s.max_message_size_in_kilobytes);
            write_policies(&mut o, &d.policies, include_keys);
            if !subscriptions.is_empty() {
                o.open("Subscriptions");
                for sub in subscriptions {
                    let s = &sub.d.settings;
                    o.open("Subscription");
                    o.leaf("Name", &sub.name);
                    o.leaf("TopicPath", name);
                    o.span("LockDuration", s.lock_duration);
                    o.flag("RequiresSession", s.requires_session);
                    o.span("DefaultMessageTimeToLive", s.default_message_time_to_live);
                    o.flag(
                        "EnableDeadLetteringOnMessageExpiration",
                        s.dead_lettering_on_message_expiration,
                    );
                    o.flag(
                        "EnableDeadLetteringOnFilterEvaluationExceptions",
                        s.dead_lettering_on_filter_evaluation_exceptions,
                    );
                    o.num("MaxDeliveryCount", s.max_delivery_count);
                    o.flag("EnableBatchedOperations", s.enable_batched_operations);
                    o.status(s.status);
                    o.text("ForwardTo", &bare(&s.forward_to));
                    o.text(
                        "ForwardDeadLetteredMessagesTo",
                        &bare(&s.forward_dead_lettered_messages_to),
                    );
                    o.span("AutoDeleteOnIdle", s.auto_delete_on_idle);
                    o.text("UserMetadata", &s.user_metadata);
                    if !sub.rules.is_empty() {
                        o.open("Rules");
                        for r in &sub.rules {
                            write_rule(&mut o, r);
                        }
                        o.close("Rules");
                    }
                    o.close("Subscription");
                }
                o.close("Subscriptions");
            }
            o.close("Topic");
        }
        o.close("Topics");
    }
    o.depth -= 1;
    o.line("</Entities>");
    o.s
}

// ---------------------------------------------------------------------------------------------
// Import

/// `node` and its descendants in the incumbent's namespace; elements of any other namespace
/// (or none) are dropped with their content.
fn qualified(node: &Node, scope: &[(String, String)]) -> Option<Node> {
    let mut scope = scope.to_vec();
    for (k, v) in &node.attrs {
        if k == "xmlns" {
            scope.push((String::new(), v.clone()));
        } else if let Some(p) = k.strip_prefix("xmlns:") {
            scope.push((p.to_string(), v.clone()));
        }
    }
    let prefix = node.prefix.clone().unwrap_or_default();
    let ns = scope
        .iter()
        .rev()
        .find(|(p, _)| *p == prefix)
        .map(|(_, v)| v.as_str())
        .unwrap_or("");
    if ns != NAMESPACE {
        return None;
    }
    let mut out = node.clone();
    out.children = node
        .children
        .iter()
        .filter_map(|c| qualified(c, &scope))
        .collect();
    Some(out)
}

/// All descendants of `node` named `name` (not looking inside a match).
fn descendants<'a>(node: &'a Node, name: &str, out: &mut Vec<&'a Node>) {
    for c in &node.children {
        if c.name == name {
            out.push(c);
        } else {
            descendants(c, name, out);
        }
    }
}

fn text(n: &Node, name: &str) -> Option<String> {
    n.child(name)
        .map(|c| c.text.clone())
        .filter(|t| !t.is_empty())
}

fn invalid(what: &str, element: &str, value: &str) -> Error {
    Error::Invalid(format!("{what}: {element} `{value}` cannot be read"))
}

fn span(n: &Node, name: &str, what: &str) -> Result<Option<Duration>> {
    text(n, name)
        .map(|t| parse_dotnet_timespan(&t).map_err(|_| invalid(what, name, &t)))
        .transpose()
}

fn flag(n: &Node, name: &str, what: &str) -> Result<Option<bool>> {
    text(n, name)
        .map(|t| match t.trim().to_ascii_lowercase().as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(invalid(what, name, &t)),
        })
        .transpose()
}

fn num<N: std::str::FromStr>(n: &Node, name: &str, what: &str) -> Result<Option<N>> {
    text(n, name)
        .map(|t| t.trim().parse().map_err(|_| invalid(what, name, &t)))
        .transpose()
}

fn status(n: &Node, what: &str) -> Result<Option<EntityStatus>> {
    text(n, "Status")
        .map(|t| t.parse().map_err(|_| invalid(what, "Status", &t)))
        .transpose()
}

/// Split an incumbent correlation property, `key: value`, on its first colon; one space after
/// the colon is dropped.
pub fn split_property(p: &str) -> Result<(String, String)> {
    let (k, v) = p.split_once(':').ok_or_else(|| {
        Error::Invalid(format!(
            "correlation property `{p}` has no `:`; write it as `key: value`"
        ))
    })?;
    Ok((k.to_string(), v.strip_prefix(' ').unwrap_or(v).to_string()))
}

fn policies(n: &Node, what: &str) -> Result<Vec<ImportPolicy>> {
    let Some(rules) = n.child("AuthorizationRules") else {
        return Ok(Vec::new());
    };
    rules
        .children_named("SharedAccessAuthorizationRule")
        .map(|r| {
            let key_name = text(r, "KeyName").ok_or_else(|| {
                Error::Invalid(format!(
                    "{what}: a SharedAccessAuthorizationRule has no KeyName"
                ))
            })?;
            let rights = r
                .child("Rights")
                .map(|rs| {
                    rs.children_named("Right")
                        .map(|x| {
                            x.text.parse::<AccessRight>().map_err(|_| {
                                invalid(&format!("{what}, policy {key_name}"), "Right", &x.text)
                            })
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default();
            // A file exported without keys has none: `None`, not a new key (see `ImportPolicy`).
            let key = |name: &str| text(r, name).map(SecretString::from);
            Ok(ImportPolicy {
                rights,
                primary_key: key("PrimaryKey"),
                secondary_key: key("SecondaryKey"),
                key_name,
            })
        })
        .collect()
}

fn rule(n: &Node, what: &str) -> Result<RuleSpec> {
    let name = text(n, "Name").unwrap_or_else(|| "$Default".into());
    let what = format!("{what}, rule {name}");
    let filter = match n.child("Filter") {
        None => RuleFilter::True,
        Some(f) => {
            if let Some(sql) = f.child("SqlFilter") {
                RuleFilter::Sql {
                    expression: text(sql, "SqlExpression").ok_or_else(|| {
                        Error::Invalid(format!("{what}: the SqlFilter has no SqlExpression"))
                    })?,
                }
            } else if let Some(c) = f.child("CorrelationFilter") {
                let properties = c
                    .child("Properties")
                    .map(|ps| {
                        ps.children_named("Property")
                            .map(|p| {
                                split_property(&p.text)
                                    .map(|(name, value)| CorrelationProperty { name, value })
                            })
                            .collect::<Result<Vec<_>>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                RuleFilter::Correlation {
                    correlation_id: text(c, "CorrelationId"),
                    message_id: text(c, "MessageId"),
                    to: text(c, "To"),
                    reply_to: text(c, "ReplyTo"),
                    subject: text(c, "Label"),
                    session_id: text(c, "SessionId"),
                    reply_to_session_id: text(c, "ReplyToSessionId"),
                    content_type: text(c, "ContentType"),
                    properties,
                }
            } else {
                // `<Filter />`: the incumbent's export of a TrueFilter (the default rule).
                RuleFilter::True
            }
        }
    };
    let action = n
        .child("Action")
        .and_then(|a| a.child("SqlRuleAction"))
        .and_then(|a| text(a, "SqlExpression"));
    Ok(RuleSpec {
        name,
        filter,
        action,
    })
}

/// Read an incumbent `Entities` file: queues, then topics, then their subscriptions (with
/// rules). Elements outside the incumbent's namespace and unknown elements are ignored;
/// TimeSpans may be .NET or ISO-8601 text, booleans any case. A value that cannot be read
/// refuses the whole file, so an import never runs on half of it.
pub fn parse_entities(xml_text: &str) -> Result<Vec<ImportEntity>> {
    let doc = xml::parse_exact(xml_text)?;
    let root = qualified(&doc, &[])
        .filter(|r| r.name == "Entities")
        .ok_or_else(|| {
            Error::Invalid(format!(
                "not an entity file: the root must be `Entities` in {NAMESPACE}"
            ))
        })?;
    let mut out = Vec::new();
    let mut queues = Vec::new();
    descendants(&root, "Queue", &mut queues);
    for q in queues {
        let name = text(q, "Path").ok_or_else(|| Error::Invalid("a Queue has no Path".into()))?;
        let what = format!("queue {name}");
        let w = what.as_str();
        out.push(ImportEntity::Queue {
            d: Description {
                settings: QueueSettings {
                    lock_duration: span(q, "LockDuration", w)?,
                    max_size_in_megabytes: num(q, "MaxSizeInMegabytes", w)?,
                    requires_duplicate_detection: flag(q, "RequiresDuplicateDetection", w)?,
                    requires_session: flag(q, "RequiresSession", w)?,
                    default_message_time_to_live: span(q, "DefaultMessageTimeToLive", w)?,
                    dead_lettering_on_message_expiration: flag(
                        q,
                        "EnableDeadLetteringOnMessageExpiration",
                        w,
                    )?,
                    duplicate_detection_history_time_window: span(
                        q,
                        "DuplicateDetectionHistoryTimeWindow",
                        w,
                    )?,
                    max_delivery_count: num(q, "MaxDeliveryCount", w)?,
                    enable_batched_operations: flag(q, "EnableBatchedOperations", w)?,
                    status: status(q, w)?,
                    forward_to: text(q, "ForwardTo"),
                    forward_dead_lettered_messages_to: text(q, "ForwardDeadLetteredMessagesTo"),
                    auto_delete_on_idle: span(q, "AutoDeleteOnIdle", w)?,
                    enable_partitioning: flag(q, "EnablePartitioning", w)?,
                    enable_express: flag(q, "EnableExpress", w)?,
                    max_message_size_in_kilobytes: num(q, "MaxMessageSizeInKilobytes", w)?,
                    user_metadata: text(q, "UserMetadata"),
                },
                policies: Vec::new(),
                raw: Node::default(),
            },
            policies: policies(q, w)?,
            name,
        });
    }
    let mut topics = Vec::new();
    descendants(&root, "Topic", &mut topics);
    let mut subscriptions = Vec::new();
    for t in topics {
        let name = text(t, "Path").ok_or_else(|| Error::Invalid("a Topic has no Path".into()))?;
        let what = format!("topic {name}");
        let w = what.as_str();
        out.push(ImportEntity::Topic {
            d: Description {
                settings: TopicSettings {
                    max_size_in_megabytes: num(t, "MaxSizeInMegabytes", w)?,
                    requires_duplicate_detection: flag(t, "RequiresDuplicateDetection", w)?,
                    duplicate_detection_history_time_window: span(
                        t,
                        "DuplicateDetectionHistoryTimeWindow",
                        w,
                    )?,
                    default_message_time_to_live: span(t, "DefaultMessageTimeToLive", w)?,
                    enable_batched_operations: flag(t, "EnableBatchedOperations", w)?,
                    status: status(t, w)?,
                    auto_delete_on_idle: span(t, "AutoDeleteOnIdle", w)?,
                    enable_partitioning: flag(t, "EnablePartitioning", w)?,
                    enable_express: flag(t, "EnableExpress", w)?,
                    support_ordering: flag(t, "SupportOrdering", w)?,
                    max_message_size_in_kilobytes: num(t, "MaxMessageSizeInKilobytes", w)?,
                    user_metadata: text(t, "UserMetadata"),
                },
                policies: Vec::new(),
                raw: Node::default(),
            },
            policies: policies(t, w)?,
            name: name.clone(),
        });
        let mut subs = Vec::new();
        descendants(t, "Subscription", &mut subs);
        for s in subs {
            let sub = text(s, "Name").ok_or_else(|| {
                Error::Invalid(format!("a Subscription of topic {name} has no Name"))
            })?;
            let topic = text(s, "TopicPath").unwrap_or_else(|| name.clone());
            let what = format!("subscription {topic}/subscriptions/{sub}");
            let w = what.as_str();
            let mut rule_nodes = Vec::new();
            descendants(s, "Rule", &mut rule_nodes);
            subscriptions.push(ImportEntity::Subscription {
                d: Description::new(SubscriptionSettings {
                    lock_duration: span(s, "LockDuration", w)?,
                    requires_session: flag(s, "RequiresSession", w)?,
                    default_message_time_to_live: span(s, "DefaultMessageTimeToLive", w)?,
                    dead_lettering_on_message_expiration: flag(
                        s,
                        "EnableDeadLetteringOnMessageExpiration",
                        w,
                    )?,
                    dead_lettering_on_filter_evaluation_exceptions: flag(
                        s,
                        "EnableDeadLetteringOnFilterEvaluationExceptions",
                        w,
                    )?,
                    max_delivery_count: num(s, "MaxDeliveryCount", w)?,
                    enable_batched_operations: flag(s, "EnableBatchedOperations", w)?,
                    status: status(s, w)?,
                    forward_to: text(s, "ForwardTo"),
                    forward_dead_lettered_messages_to: text(s, "ForwardDeadLetteredMessagesTo"),
                    auto_delete_on_idle: span(s, "AutoDeleteOnIdle", w)?,
                    user_metadata: text(s, "UserMetadata"),
                }),
                rules: rule_nodes
                    .into_iter()
                    .map(|r| rule(r, w))
                    .collect::<Result<_>>()?,
                topic,
                name: sub,
            });
        }
    }
    out.extend(subscriptions);
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Collecting

impl Explorer {
    /// The entities to export: `selection`, or every queue and topic when it is empty. A
    /// selected subscription is exported under its topic; a topic brings all its
    /// subscriptions and their rules.
    pub async fn collect_for_export(&self, selection: &[EntityRef]) -> Result<Vec<ExportEntity>> {
        let m = self.management_client();
        let mut wanted: Vec<EntityRef> = selection.to_vec();
        if wanted.is_empty() {
            for q in m.list_queues().await? {
                wanted.push(EntityRef::queue(&q.name));
            }
            for t in m.list_topics().await? {
                wanted.push(EntityRef::topic(&t.name));
            }
        }
        let mut out: Vec<ExportEntity> = Vec::new();
        for e in wanted {
            match e {
                EntityRef::Queue { name } => {
                    let d = m.get_queue_description(&name).await?;
                    if !out
                        .iter()
                        .any(|x| matches!(x, ExportEntity::Queue { name: n, .. } if *n == name))
                    {
                        out.push(ExportEntity::Queue { name, d });
                    }
                }
                EntityRef::Topic { name } => {
                    let subs: Vec<String> = m
                        .list_subscriptions(&name)
                        .await?
                        .into_iter()
                        .map(|s| s.name)
                        .collect();
                    for s in subs {
                        self.add_subscription(&mut out, &name, &s).await?;
                    }
                    self.topic_slot(&mut out, &name).await?;
                }
                EntityRef::Subscription { topic, name } => {
                    self.add_subscription(&mut out, &topic, &name).await?;
                }
            }
        }
        Ok(out)
    }

    /// The index of topic `name` in `out`, reading it the first time.
    async fn topic_slot(&self, out: &mut Vec<ExportEntity>, name: &str) -> Result<usize> {
        if let Some(i) = out
            .iter()
            .position(|x| matches!(x, ExportEntity::Topic { name: n, .. } if n == name))
        {
            return Ok(i);
        }
        let d = self.management_client().get_topic_description(name).await?;
        out.push(ExportEntity::Topic {
            name: name.to_string(),
            d,
            subscriptions: Vec::new(),
        });
        Ok(out.len() - 1)
    }

    async fn add_subscription(
        &self,
        out: &mut Vec<ExportEntity>,
        topic: &str,
        name: &str,
    ) -> Result<()> {
        let m = self.management_client();
        let at = self.topic_slot(out, topic).await?;
        if let ExportEntity::Topic { subscriptions, .. } = &out[at]
            && subscriptions.iter().any(|s| s.name == name)
        {
            return Ok(());
        }
        let d = m.get_subscription(topic, name).await?;
        let mut rules = Vec::new();
        for r in m.list_rules(topic, name).await? {
            rules.push(RuleSpec {
                name: r.name,
                filter: r.filter,
                action: r.action,
            });
        }
        if let ExportEntity::Topic { subscriptions, .. } = &mut out[at] {
            subscriptions.push(ExportSubscription {
                name: name.to_string(),
                d,
                rules,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mgmt::{AccessPolicy, AccessRight, CorrelationProperty, EntityStatus, TimeSpan};
    use secrecy::{ExposeSecret, SecretString};
    use std::time::Duration;

    fn sample() -> Vec<ExportEntity> {
        let mut q = Description::new(QueueSettings {
            lock_duration: Some(Duration::from_secs(60)),
            max_size_in_megabytes: Some(1024),
            requires_session: Some(false),
            default_message_time_to_live: Some(Duration::from_secs(14 * 86_400)),
            dead_lettering_on_message_expiration: Some(true),
            max_delivery_count: Some(10),
            status: Some(EntityStatus::Active),
            forward_to: Some("sb://contoso.servicebus.windows.net/archive".into()),
            auto_delete_on_idle: Some(TimeSpan::MAX),
            user_metadata: Some("a <b> & c".into()),
            ..Default::default()
        });
        q.policies.push(AccessPolicy {
            key_name: "app".into(),
            rights: vec![AccessRight::Listen, AccessRight::Send],
            primary_key: SecretString::from("PRIMARY-1"),
            secondary_key: SecretString::from("SECONDARY-2"),
        });
        let t = Description::new(TopicSettings {
            max_size_in_megabytes: Some(1024),
            default_message_time_to_live: Some(TimeSpan::MAX),
            support_ordering: Some(true),
            ..Default::default()
        });
        let s = Description::new(SubscriptionSettings {
            lock_duration: Some(Duration::from_secs(30)),
            max_delivery_count: Some(5),
            status: Some(EntityStatus::ReceiveDisabled),
            ..Default::default()
        });
        vec![
            ExportEntity::Queue {
                name: "orders".into(),
                d: q,
            },
            ExportEntity::Topic {
                name: "events".into(),
                d: t,
                subscriptions: vec![ExportSubscription {
                    name: "billing".into(),
                    d: s,
                    rules: vec![
                        RuleSpec {
                            name: "$Default".into(),
                            filter: RuleFilter::True,
                            action: None,
                        },
                        RuleSpec {
                            name: "eu".into(),
                            filter: RuleFilter::Sql {
                                expression: "region = 'eu'".into(),
                            },
                            action: Some("SET tagged = 1".into()),
                        },
                        RuleSpec {
                            name: "by-corr".into(),
                            filter: RuleFilter::Correlation {
                                correlation_id: Some("abc".into()),
                                message_id: None,
                                to: None,
                                reply_to: None,
                                subject: Some("order.created".into()),
                                session_id: None,
                                reply_to_session_id: None,
                                content_type: None,
                                properties: vec![CorrelationProperty {
                                    name: "tenant".into(),
                                    value: "acme".into(),
                                }],
                            },
                            action: None,
                        },
                    ],
                }],
            },
        ]
    }

    #[test]
    fn dotnet_timespans_round_trip() {
        for (text, d) in [
            ("00:01:00", Duration::from_secs(60)),
            ("14.00:00:00", Duration::from_secs(14 * 86_400)),
            ("00:00:30.5000000", Duration::from_millis(30_500)),
            ("10675199.02:48:05.4775807", TimeSpan::MAX),
        ] {
            assert_eq!(dotnet_timespan(d), text);
            assert_eq!(parse_dotnet_timespan(text).unwrap(), d, "{text}");
        }
        assert_eq!(
            parse_dotnet_timespan("PT1M").unwrap(),
            Duration::from_secs(60),
            "ISO-8601 is accepted too"
        );
        assert_eq!(
            parse_dotnet_timespan("1.02:03:04.5").unwrap(),
            Duration::new(93_784, 500_000_000)
        );
        assert_eq!(
            parse_dotnet_timespan("2").unwrap(),
            Duration::from_secs(2 * 86_400)
        );
        for bad in ["", "abc", "-00:01:00", "00:61:00", "1:2"] {
            assert!(parse_dotnet_timespan(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn export_matches_the_incumbent_shape() {
        let xml = export_entities("contoso-dev", &sample(), true);
        assert!(xml.starts_with(
            "<Entities xmlns=\"http://schemas.microsoft.com/servicebusexplorer\" serviceBusNamespace=\"contoso-dev\">"
        ), "{xml}");
        for want in [
            "<Path>orders</Path>",
            "<LockDuration>00:01:00</LockDuration>",
            "<DefaultMessageTimeToLive>14.00:00:00</DefaultMessageTimeToLive>",
            "<AutoDeleteOnIdle>10675199.02:48:05.4775807</AutoDeleteOnIdle>",
            "<RequiresSession>False</RequiresSession>",
            "<EnableDeadLetteringOnMessageExpiration>True</EnableDeadLetteringOnMessageExpiration>",
            "<ForwardTo>archive</ForwardTo>",
            "<UserMetadata>a &lt;b&gt; &amp; c</UserMetadata>",
            "<Rights><Right>Listen</Right><Right>Send</Right></Rights>",
            "<KeyName>app</KeyName>",
            "<PrimaryKey>PRIMARY-1</PrimaryKey>",
            "<Name>billing</Name>",
            "<TopicPath>events</TopicPath>",
            "<Status>ReceiveDisabled</Status>",
            "<SupportOrdering>True</SupportOrdering>",
            "<SqlFilter><SqlExpression>region = 'eu'</SqlExpression></SqlFilter>",
            "<SqlRuleAction><SqlExpression>SET tagged = 1</SqlExpression></SqlRuleAction>",
            "<Property>tenant: acme</Property>",
            "<Label>order.created</Label>",
            "<MessageId />",
        ] {
            assert!(xml.contains(want), "missing {want} in\n{xml}");
        }
        // Order inside an entity: scalars, then AuthorizationRules, then Subscriptions.
        let at = |s: &str| xml.find(s).unwrap();
        assert!(at("<Path>orders</Path>") < at("<LockDuration>"));
        assert!(at("<Path>events</Path>") < at("<Subscriptions>"));
        assert!(at("<Name>billing</Name>") < at("<TopicPath>events</TopicPath>"));
        assert!(
            !xml.contains("<?xml"),
            "the incumbent writes no declaration"
        );
    }

    #[test]
    fn export_omits_keys_unless_asked() {
        let xml = export_entities("ns", &sample(), false);
        assert!(xml.contains("<KeyName>app</KeyName>"));
        assert!(
            !xml.contains("PRIMARY-1") && !xml.contains("SECONDARY-2"),
            "{xml}"
        );
        assert!(!xml.contains("<PrimaryKey>"));
        let with = export_entities("ns", &sample(), true);
        assert!(with.contains("<SecondaryKey>SECONDARY-2</SecondaryKey>"));
    }

    #[test]
    fn parse_reads_the_incumbent_example() {
        let items =
            parse_entities(include_str!("../../tests/fixtures/incumbent_entities.xml")).unwrap();
        let [
            ImportEntity::Queue {
                name: q,
                d: qd,
                policies: qp,
            },
            ImportEntity::Topic { name: t, d: td, .. },
            ImportEntity::Subscription {
                topic,
                name: s,
                d: sd,
                rules,
            },
        ] = items.as_slice()
        else {
            panic!("{items:#?}")
        };
        assert_eq!((q.as_str(), t.as_str()), ("orders", "events"));
        assert_eq!(qd.settings.lock_duration, Some(Duration::from_secs(60)));
        assert_eq!(
            qd.settings.default_message_time_to_live,
            Some(Duration::from_secs(14 * 86_400))
        );
        assert_eq!(qd.settings.auto_delete_on_idle, Some(TimeSpan::MAX));
        assert_eq!(qd.settings.requires_session, Some(false));
        assert_eq!(qd.settings.enable_batched_operations, Some(true));
        assert_eq!(qd.settings.status, Some(EntityStatus::Active));
        assert_eq!(
            qd.settings.forward_to, None,
            "an empty ForwardTo is not set"
        );
        assert!(qd.policies.is_empty(), "the file's policies are apart");
        assert_eq!(qp.len(), 1);
        assert_eq!(qp[0].rights, [AccessRight::Send, AccessRight::Listen]);
        assert_eq!(
            qp[0].primary_key.as_ref().map(|k| k.expose_secret()),
            Some("BASE64KEY==")
        );
        assert_eq!(
            td.settings.default_message_time_to_live,
            Some(TimeSpan::MAX)
        );
        assert_eq!((topic.as_str(), s.as_str()), ("events", "billing"));
        assert_eq!(sd.settings.max_delivery_count, Some(10));
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].name, "$Default");
        assert_eq!(rules[0].filter, RuleFilter::True);
        assert_eq!(rules[0].action, None);
        assert_eq!(
            rules[1].filter,
            RuleFilter::Sql {
                expression: "region = 'eu'".into()
            }
        );
        assert_eq!(rules[1].action.as_deref(), Some("SET tagged = 1"));
        let RuleFilter::Correlation {
            correlation_id,
            message_id,
            subject,
            properties,
            ..
        } = &rules[2].filter
        else {
            panic!("{:?}", rules[2])
        };
        assert_eq!(correlation_id.as_deref(), Some("abc"));
        assert_eq!(message_id, &None);
        assert_eq!(subject.as_deref(), Some("order.created"));
        assert_eq!(
            properties,
            &[CorrelationProperty {
                name: "tenant".into(),
                value: "acme".into()
            }]
        );
    }

    #[test]
    fn parse_property_splits_on_the_first_colon() {
        assert_eq!(
            split_property("url: http://x").unwrap(),
            ("url".to_string(), "http://x".to_string())
        );
        assert_eq!(
            split_property("k:v").unwrap(),
            ("k".to_string(), "v".to_string())
        );
        assert_eq!(
            split_property("k:  two spaces").unwrap(),
            ("k".to_string(), " two spaces".to_string()),
            "one leading space is trimmed"
        );
        assert!(split_property("no colon").is_err());
    }

    #[test]
    fn parse_ignores_unqualified_elements_and_unknown_ones() {
        let xml = r#"<Entities xmlns="http://schemas.microsoft.com/servicebusexplorer" xmlns:o="urn:other" serviceBusNamespace="x">
            <Queues>
              <Queue><Path>a</Path><Shiny>yes</Shiny><o:MaxDeliveryCount>99</o:MaxDeliveryCount><MaxDeliveryCount>3</MaxDeliveryCount><RequiresSession>TRUE</RequiresSession><LockDuration>PT30S</LockDuration></Queue>
              <o:Queue><o:Path>other-ns</o:Path></o:Queue>
              <Queue xmlns=""><Path>no-ns</Path></Queue>
            </Queues>
            <Relays><Relay><Path>r</Path></Relay></Relays>
        </Entities>"#;
        let items = parse_entities(xml).unwrap();
        assert_eq!(items.len(), 1, "{items:#?}");
        let ImportEntity::Queue { name, d, .. } = &items[0] else {
            panic!()
        };
        assert_eq!(name, "a");
        assert_eq!(d.settings.max_delivery_count, Some(3));
        assert_eq!(d.settings.requires_session, Some(true));
        assert_eq!(d.settings.lock_duration, Some(Duration::from_secs(30)));
        assert!(
            parse_entities("<Entities><Queues/></Entities>").is_err(),
            "wrong namespace"
        );
    }

    #[test]
    fn export_then_parse_round_trips() {
        let mut entities = sample();
        if let ExportEntity::Queue { d, .. } = &mut entities[0] {
            d.settings.forward_to = Some("archive".into());
        }
        let xml = export_entities("ns", &entities, true);
        let items = parse_entities(&xml).unwrap();
        let ExportEntity::Queue { d: q, .. } = &entities[0] else {
            panic!()
        };
        let ExportEntity::Topic {
            d: t,
            subscriptions,
            ..
        } = &entities[1]
        else {
            panic!()
        };
        let [
            ImportEntity::Queue {
                d: q2,
                policies: p2,
                ..
            },
            ImportEntity::Topic { d: t2, .. },
            ImportEntity::Subscription { d: s2, rules, .. },
        ] = items.as_slice()
        else {
            panic!("{items:#?}")
        };
        assert_eq!(q2.settings, q.settings);
        let applied: Vec<AccessPolicy> = p2.iter().map(|p| p.applied(None)).collect();
        assert_eq!(applied, q.policies, "the keys written are the keys read");
        assert_eq!(t2.settings, t.settings);
        assert_eq!(s2.settings, subscriptions[0].d.settings);
        assert_eq!(rules, &subscriptions[0].rules);
    }
}
