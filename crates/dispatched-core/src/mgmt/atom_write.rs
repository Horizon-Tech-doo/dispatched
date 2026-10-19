//! ATOM entries for create and update requests.
//!
//! The service reads a `*Description` as a DataContract, so its elements must come in the
//! schema's sequence order. Each kind has a fixed `ORDER`. The writer emits the modelled
//! settings in that order; elements of the order it does not model are copied from the
//! description read before (`Description::raw`); elements it does not know at all are kept at
//! their original place, after the known element they followed. Read-only elements (counts,
//! timestamps) are left out.

use super::model::{
    AccessPolicy, Description, QueueSettings, READ_ONLY, RuleSpec, SubscriptionSettings,
    TopicSettings, iso_duration,
};
use super::rules::RuleFilter;
use super::xml::Node;
use quick_xml::escape::{escape, partial_escape};
use secrecy::ExposeSecret;
use std::time::Duration;

const ATOM_NS: &str = "http://www.w3.org/2005/Atom";
const SB_NS: &str = "http://schemas.microsoft.com/netservices/2010/10/servicebus/connect";
const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";
const XSD_NS: &str = "http://www.w3.org/2001/XMLSchema";

// The orders below are the sequence the service's DataContract expects, as the Azure SDK for
// .NET writes it (the REST reference on Microsoft Learn lists the properties but not their
// order).

/// Source: Azure.Messaging.ServiceBus `QueuePropertiesExtensions.Serialize`,
/// https://github.com/Azure/azure-sdk-for-net/blob/main/sdk/servicebus/Azure.Messaging.ServiceBus/src/Administration/QueuePropertiesExtensions.cs
/// (see also https://learn.microsoft.com/rest/api/servicebus/create-queue).
pub(crate) const QUEUE_ORDER: &[&str] = &[
    "LockDuration",
    "MaxSizeInMegabytes",
    "RequiresDuplicateDetection",
    "RequiresSession",
    "DefaultMessageTimeToLive",
    "DeadLetteringOnMessageExpiration",
    "DuplicateDetectionHistoryTimeWindow",
    "MaxDeliveryCount",
    "EnableBatchedOperations",
    "IsAnonymousAccessible",
    "AuthorizationRules",
    "Status",
    "ForwardTo",
    "UserMetadata",
    "SupportOrdering",
    "AutoDeleteOnIdle",
    "EnablePartitioning",
    "ForwardDeadLetteredMessagesTo",
    "EnableExpress",
    "MaxMessageSizeInKilobytes",
];

/// Source: Azure.Messaging.ServiceBus `TopicPropertiesExtensions.Serialize`,
/// https://github.com/Azure/azure-sdk-for-net/blob/main/sdk/servicebus/Azure.Messaging.ServiceBus/src/Administration/TopicPropertiesExtensions.cs
/// (see also https://learn.microsoft.com/rest/api/servicebus/create-topic).
pub(crate) const TOPIC_ORDER: &[&str] = &[
    "DefaultMessageTimeToLive",
    "MaxSizeInMegabytes",
    "RequiresDuplicateDetection",
    "DuplicateDetectionHistoryTimeWindow",
    "EnableBatchedOperations",
    "FilteringMessagesBeforePublishing",
    "IsAnonymousAccessible",
    "AuthorizationRules",
    "Status",
    "ForwardTo",
    "UserMetadata",
    "SupportOrdering",
    "AutoDeleteOnIdle",
    "EnablePartitioning",
    "EnableSubscriptionPartitioning",
    "EnableExpress",
    "MaxMessageSizeInKilobytes",
];

/// Source: Azure.Messaging.ServiceBus `SubscriptionPropertiesExtensions.Serialize`,
/// https://github.com/Azure/azure-sdk-for-net/blob/main/sdk/servicebus/Azure.Messaging.ServiceBus/src/Administration/SubscriptionPropertiesExtensions.cs
/// (see also https://learn.microsoft.com/rest/api/servicebus/create-subscription).
pub(crate) const SUBSCRIPTION_ORDER: &[&str] = &[
    "LockDuration",
    "RequiresSession",
    "DefaultMessageTimeToLive",
    "DeadLetteringOnMessageExpiration",
    "DeadLetteringOnFilterEvaluationExceptions",
    "DefaultRuleDescription",
    "MaxDeliveryCount",
    "EnableBatchedOperations",
    "Status",
    "ForwardTo",
    "UserMetadata",
    "ForwardDeadLetteredMessagesTo",
    "AutoDeleteOnIdle",
];

/// Source: Azure.Messaging.ServiceBus `RuleDescriptionExtensions.SerializeRule`,
/// https://github.com/Azure/azure-sdk-for-net/blob/main/sdk/servicebus/Azure.Messaging.ServiceBus/src/Administration/Rules/RuleDescriptionExtensions.cs
/// (see also https://learn.microsoft.com/rest/api/servicebus/create-rule).
pub(crate) const RULE_ORDER: &[&str] = &["Filter", "Action", "Name"];

/// Source: Azure.Messaging.ServiceBus `CorrelationRuleFilterExtensions.Serialize`,
/// https://github.com/Azure/azure-sdk-for-net/blob/main/sdk/servicebus/Azure.Messaging.ServiceBus/src/Administration/Rules/CorrelationRuleFilterExtensions.cs
pub(crate) const CORRELATION_ORDER: &[&str] = &[
    "CorrelationId",
    "MessageId",
    "To",
    "ReplyTo",
    "Label",
    "SessionId",
    "ReplyToSessionId",
    "ContentType",
    "Properties",
];

/// Value types a correlation property may carry (`XmlObjectConvertor.SerializeObject`).
const VALUE_TYPES: &[&str] = &[
    "string", "int", "long", "boolean", "double", "dateTime", "duration",
];

/// Element `name` with escaped text.
fn leaf(name: &str, value: &str) -> String {
    format!("<{name}>{}</{name}>", partial_escape(value))
}

/// The modelled elements of a description, each already written.
struct Known(Vec<(&'static str, Option<String>)>);

impl Known {
    fn new() -> Self {
        Self(Vec::new())
    }
    fn text(&mut self, name: &'static str, v: &Option<String>) -> &mut Self {
        self.0.push((name, v.as_deref().map(|v| leaf(name, v))));
        self
    }
    fn dur(&mut self, name: &'static str, v: Option<Duration>) -> &mut Self {
        self.0.push((name, v.map(|d| leaf(name, &iso_duration(d)))));
        self
    }
    fn flag(&mut self, name: &'static str, v: Option<bool>) -> &mut Self {
        self.0.push((
            name,
            v.map(|b| leaf(name, if b { "true" } else { "false" })),
        ));
        self
    }
    fn num<N: ToString>(&mut self, name: &'static str, v: Option<N>) -> &mut Self {
        self.0.push((name, v.map(|n| leaf(name, &n.to_string()))));
        self
    }
    fn xml(&mut self, name: &'static str, v: Option<String>) -> &mut Self {
        self.0.push((name, v));
        self
    }
}

/// Write `node` as it was read: prefix, attributes, text and children.
pub(crate) fn write_node(out: &mut String, node: &Node) {
    let qname = match &node.prefix {
        Some(p) => format!("{p}:{}", node.name),
        None => node.name.clone(),
    };
    out.push('<');
    out.push_str(&qname);
    for (k, v) in &node.attrs {
        out.push_str(&format!(" {k}=\"{}\"", escape(v.as_str())));
    }
    if node.text.is_empty() && node.children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    out.push_str(&partial_escape(node.text.as_str()));
    for c in &node.children {
        write_node(out, c);
    }
    out.push_str(&format!("</{qname}>"));
}

/// The children of a description in `order`: modelled values from `known`, the other
/// elements of the order copied from `raw`, unknown elements of `raw` after the known element
/// they followed.
fn children(order: &[&str], known: &Known, raw: &Node) -> String {
    let modelled = |name: &str| known.0.iter().find(|(n, _)| *n == name);
    // Unknown raw elements, each with the index in `order` of the known element before it.
    let mut anchored: Vec<(Option<usize>, &Node)> = Vec::new();
    let mut anchor = None;
    for c in &raw.children {
        if let Some(i) = order.iter().position(|n| *n == c.name) {
            anchor = Some(i);
        } else if !READ_ONLY.contains(&c.name.as_str()) {
            anchored.push((anchor, c));
        }
    }
    let mut out = String::new();
    let unknown_after = |out: &mut String, at: Option<usize>| {
        for (_, n) in anchored.iter().filter(|(a, _)| *a == at) {
            write_node(out, n);
        }
    };
    unknown_after(&mut out, None);
    for (i, name) in order.iter().enumerate() {
        match modelled(name) {
            Some((_, Some(xml))) => out.push_str(xml),
            Some((_, None)) => {}
            None => {
                if let Some(n) = raw.child(name) {
                    write_node(&mut out, n);
                }
            }
        }
        unknown_after(&mut out, Some(i));
    }
    out
}

/// The opening tag of a description: the Service Bus default namespace, `i:` and any other
/// prefix the read description declared (its unknown elements may use them).
fn open_description(tag: &str, raw: &Node) -> String {
    let mut s = format!("<{tag} xmlns=\"{SB_NS}\" xmlns:i=\"{XSI_NS}\"");
    for (k, v) in &raw.attrs {
        if k.starts_with("xmlns:") && k != "xmlns:i" {
            s.push_str(&format!(" {k}=\"{}\"", escape(v.as_str())));
        }
    }
    s.push('>');
    s
}

fn entry(description: String) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><entry xmlns=\"{ATOM_NS}\"><content type=\"application/xml\">{description}</content></entry>"
    )
}

fn policies_xml(policies: &[AccessPolicy]) -> String {
    let mut s = String::from("<AuthorizationRules>");
    for p in policies {
        s.push_str("<AuthorizationRule i:type=\"SharedAccessAuthorizationRule\">");
        s.push_str(&leaf("ClaimType", "SharedAccessKey"));
        s.push_str(&leaf("ClaimValue", "None"));
        s.push_str("<Rights>");
        for r in &p.rights {
            s.push_str(&leaf("AccessRights", r.as_str()));
        }
        s.push_str("</Rights>");
        s.push_str(&leaf("KeyName", &p.key_name));
        s.push_str(&leaf("PrimaryKey", p.primary_key.expose_secret()));
        s.push_str(&leaf("SecondaryKey", p.secondary_key.expose_secret()));
        s.push_str("</AuthorizationRule>");
    }
    s.push_str("</AuthorizationRules>");
    s
}

/// The `QueueDescription` element.
fn queue_description(d: &Description<QueueSettings>) -> String {
    let s = &d.settings;
    let mut k = Known::new();
    k.dur("LockDuration", s.lock_duration)
        .num("MaxSizeInMegabytes", s.max_size_in_megabytes)
        .flag("RequiresDuplicateDetection", s.requires_duplicate_detection)
        .flag("RequiresSession", s.requires_session)
        .dur("DefaultMessageTimeToLive", s.default_message_time_to_live)
        .flag(
            "DeadLetteringOnMessageExpiration",
            s.dead_lettering_on_message_expiration,
        )
        .dur(
            "DuplicateDetectionHistoryTimeWindow",
            s.duplicate_detection_history_time_window,
        )
        .num("MaxDeliveryCount", s.max_delivery_count)
        .flag("EnableBatchedOperations", s.enable_batched_operations)
        .xml("AuthorizationRules", Some(policies_xml(&d.policies)))
        .text("Status", &s.status.map(|v| v.as_str().to_string()))
        .text("ForwardTo", &s.forward_to)
        .text("UserMetadata", &s.user_metadata)
        .dur("AutoDeleteOnIdle", s.auto_delete_on_idle)
        .flag("EnablePartitioning", s.enable_partitioning)
        .text(
            "ForwardDeadLetteredMessagesTo",
            &s.forward_dead_lettered_messages_to,
        )
        .flag("EnableExpress", s.enable_express)
        .num("MaxMessageSizeInKilobytes", s.max_message_size_in_kilobytes);
    format!(
        "{}{}</QueueDescription>",
        open_description("QueueDescription", &d.raw),
        children(QUEUE_ORDER, &k, &d.raw)
    )
}

/// The full ATOM entry to create or update a queue.
pub fn queue_entry(d: &Description<QueueSettings>) -> String {
    entry(queue_description(d))
}

/// The full ATOM entry to create or update a topic.
pub fn topic_entry(d: &Description<TopicSettings>) -> String {
    let s = &d.settings;
    let mut k = Known::new();
    k.dur("DefaultMessageTimeToLive", s.default_message_time_to_live)
        .num("MaxSizeInMegabytes", s.max_size_in_megabytes)
        .flag("RequiresDuplicateDetection", s.requires_duplicate_detection)
        .dur(
            "DuplicateDetectionHistoryTimeWindow",
            s.duplicate_detection_history_time_window,
        )
        .flag("EnableBatchedOperations", s.enable_batched_operations)
        .xml("AuthorizationRules", Some(policies_xml(&d.policies)))
        .text("Status", &s.status.map(|v| v.as_str().to_string()))
        .text("UserMetadata", &s.user_metadata)
        .flag("SupportOrdering", s.support_ordering)
        .dur("AutoDeleteOnIdle", s.auto_delete_on_idle)
        .flag("EnablePartitioning", s.enable_partitioning)
        .flag("EnableExpress", s.enable_express)
        .num("MaxMessageSizeInKilobytes", s.max_message_size_in_kilobytes);
    entry(format!(
        "{}{}</TopicDescription>",
        open_description("TopicDescription", &d.raw),
        children(TOPIC_ORDER, &k, &d.raw)
    ))
}

/// The full ATOM entry to update a subscription, or to create one with the service's default
/// rule.
pub fn subscription_entry(d: &Description<SubscriptionSettings>) -> String {
    subscription_entry_with_rule(d, None)
}

/// The full ATOM entry to create a subscription whose initial rule is `rule`
/// (`DefaultRuleDescription`) instead of the service's `$Default`. The rule is written through
/// its `raw` element, as [`rule_entry`] writes one, so a rule read from the service keeps its
/// correlation value types (int, long, …) and unknown elements.
pub fn subscription_entry_with_rule(
    d: &Description<SubscriptionSettings>,
    rule: Option<&Description<RuleSpec>>,
) -> String {
    let s = &d.settings;
    let mut k = Known::new();
    k.dur("LockDuration", s.lock_duration)
        .flag("RequiresSession", s.requires_session)
        .dur("DefaultMessageTimeToLive", s.default_message_time_to_live)
        .flag(
            "DeadLetteringOnMessageExpiration",
            s.dead_lettering_on_message_expiration,
        )
        .flag(
            "DeadLetteringOnFilterEvaluationExceptions",
            s.dead_lettering_on_filter_evaluation_exceptions,
        )
        .xml(
            "DefaultRuleDescription",
            rule.map(|r| rule_element("DefaultRuleDescription", &r.settings, Some(&r.raw))),
        )
        .num("MaxDeliveryCount", s.max_delivery_count)
        .flag("EnableBatchedOperations", s.enable_batched_operations)
        .text("Status", &s.status.map(|v| v.as_str().to_string()))
        .text("ForwardTo", &s.forward_to)
        .text("UserMetadata", &s.user_metadata)
        .text(
            "ForwardDeadLetteredMessagesTo",
            &s.forward_dead_lettered_messages_to,
        )
        .dur("AutoDeleteOnIdle", s.auto_delete_on_idle);
    entry(format!(
        "{}{}</SubscriptionDescription>",
        open_description("SubscriptionDescription", &d.raw),
        children(SUBSCRIPTION_ORDER, &k, &d.raw)
    ))
}

/// The full ATOM entry to create or update a rule.
pub fn rule_entry(d: &Description<RuleSpec>) -> String {
    entry(rule_root(d))
}

fn rule_root(d: &Description<RuleSpec>) -> String {
    let body = rule_body(&d.settings, Some(&d.raw));
    format!(
        "{}{}</RuleDescription>",
        open_description("RuleDescription", &d.raw),
        body
    )
}

/// A rule nested in a subscription (`DefaultRuleDescription`).
fn rule_element(tag: &str, r: &RuleSpec, raw: Option<&Node>) -> String {
    format!("<{tag}>{}</{tag}>", rule_body(r, raw))
}

fn rule_body(r: &RuleSpec, raw: Option<&Node>) -> String {
    let raw_filter = raw.and_then(|n| n.child("Filter"));
    let mut k = Known::new();
    k.xml("Filter", filter_xml(&r.filter, raw_filter))
        .xml(
            "Action",
            r.action.as_ref().map(|a| {
                format!(
                    "<Action i:type=\"SqlRuleAction\">{}</Action>",
                    leaf("SqlExpression", a)
                )
            }),
        )
        .text("Name", &Some(r.name.clone()));
    let empty = Node::default();
    children(RULE_ORDER, &k, raw.unwrap_or(&empty))
}

/// The `Filter` element with its `i:type`. A correlation property keeps the value type it was
/// read with (when its value is unchanged); a new one is a string. An unknown filter is
/// written back as it was read, or not at all.
fn filter_xml(f: &RuleFilter, raw: Option<&Node>) -> Option<String> {
    let sql = |ty: &str, e: &str| {
        format!(
            "<Filter i:type=\"{ty}\">{}</Filter>",
            leaf("SqlExpression", e)
        )
    };
    match f {
        RuleFilter::True => Some(sql("TrueFilter", "1=1")),
        RuleFilter::Sql { expression } => Some(sql("SqlFilter", expression)),
        RuleFilter::Unknown => raw.map(|n| {
            let mut s = String::new();
            write_node(&mut s, n);
            s
        }),
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
            let mut s = String::from("<Filter i:type=\"CorrelationFilter\">");
            let fields = [
                ("CorrelationId", correlation_id),
                ("MessageId", message_id),
                ("To", to),
                ("ReplyTo", reply_to),
                ("Label", subject),
                ("SessionId", session_id),
                ("ReplyToSessionId", reply_to_session_id),
                ("ContentType", content_type),
            ];
            debug_assert!(
                fields
                    .iter()
                    .map(|(n, _)| *n)
                    .eq(CORRELATION_ORDER[..8].iter().copied())
            );
            for (name, v) in fields {
                if let Some(v) = v.as_deref().filter(|v| !v.trim().is_empty()) {
                    s.push_str(&leaf(name, v));
                }
            }
            s.push_str("<Properties>");
            let raw_props = raw.and_then(|n| n.child("Properties"));
            for p in properties {
                let ty = raw_props
                    .and_then(|rp| {
                        rp.children_named("KeyValueOfstringanyType").find(|kv| {
                            kv.child("Key").map(|k| k.text.as_str()) == Some(p.name.as_str())
                                && kv.child("Value").map(|v| v.text.as_str())
                                    == Some(p.value.as_str())
                        })
                    })
                    .and_then(|kv| kv.child("Value"))
                    .and_then(Node::xsi_type)
                    .filter(|t| VALUE_TYPES.contains(t))
                    .unwrap_or("string");
                s.push_str(&format!(
                    "<KeyValueOfstringanyType>{}<Value i:type=\"d6p1:{ty}\" xmlns:d6p1=\"{XSD_NS}\">{}</Value></KeyValueOfstringanyType>",
                    leaf("Key", &p.name),
                    partial_escape(p.value.as_str())
                ));
            }
            s.push_str("</Properties></Filter>");
            Some(s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mgmt::model::{
        AccessRight, EntityStatus, TimeSpan, parse_queue, parse_rule, parse_subscription,
        parse_topic, validate_filter,
    };
    use crate::mgmt::rules::CorrelationProperty;
    use crate::mgmt::xml;
    use secrecy::SecretString;

    fn full_queue() -> Description<QueueSettings> {
        let entry =
            xml::parse_exact(include_str!("../../tests/fixtures/queue_entry_full.xml")).unwrap();
        parse_queue(&entry).unwrap()
    }

    /// The local names of the description's children, in order.
    fn element_names(entry_xml: &str, tag: &str) -> Vec<String> {
        let root = xml::parse(entry_xml).unwrap();
        root.path(&["content", tag])
            .unwrap()
            .children
            .iter()
            .map(|c| c.name.clone())
            .collect()
    }

    fn in_order(names: &[String], order: &[&str]) -> bool {
        let idx: Vec<usize> = names
            .iter()
            .filter_map(|n| order.iter().position(|o| o == n))
            .collect();
        idx.windows(2).all(|w| w[0] < w[1])
    }

    #[test]
    fn queue_entry_round_trips_every_setting() {
        let d = full_queue();
        let s = &d.settings;
        assert_eq!(s.lock_duration, Some(Duration::from_secs(45)));
        assert_eq!(s.max_size_in_megabytes, Some(2048));
        assert_eq!(s.requires_duplicate_detection, Some(true));
        assert_eq!(s.requires_session, Some(true));
        assert_eq!(s.default_message_time_to_live, Some(TimeSpan::MAX));
        assert_eq!(s.dead_lettering_on_message_expiration, Some(true));
        assert_eq!(
            s.duplicate_detection_history_time_window,
            Some(Duration::from_secs(20 * 60))
        );
        assert_eq!(s.max_delivery_count, Some(7));
        assert_eq!(s.enable_batched_operations, Some(false));
        assert_eq!(s.status, Some(EntityStatus::SendDisabled));
        assert_eq!(s.forward_to.as_deref(), Some("archive"));
        assert_eq!(
            s.forward_dead_lettered_messages_to.as_deref(),
            Some("dlq-sink")
        );
        assert_eq!(s.auto_delete_on_idle, Some(Duration::from_secs(7 * 86_400)));
        assert_eq!(s.enable_partitioning, Some(false));
        assert_eq!(s.enable_express, Some(false));
        assert_eq!(s.max_message_size_in_kilobytes, Some(1024));
        assert_eq!(s.user_metadata.as_deref(), Some("owner: team <a&b>"));
        assert_eq!(d.policies.len(), 1);
        assert_eq!(
            d.policies[0].rights,
            [AccessRight::Listen, AccessRight::Send]
        );

        let written = queue_entry(&d);
        let again = parse_queue(&xml::parse_exact(&written).unwrap()).unwrap();
        assert_eq!(again.settings, d.settings);
        assert_eq!(again.policies, d.policies);
        let names = element_names(&written, "QueueDescription");
        assert!(in_order(&names, QUEUE_ORDER), "{names:?}");
        for gone in ["SizeInBytes", "CountDetails", "CreatedAt", "MessageCount"] {
            assert!(!names.iter().any(|n| n == gone), "{gone} is read-only");
        }
        assert!(
            names.iter().any(|n| n == "IsAnonymousAccessible"),
            "an unmodelled element of the schema is kept"
        );
        assert!(written.contains("<DefaultMessageTimeToLive>P10675199DT2H48M5.4775807S<"));
        assert!(written.contains("<LockDuration>PT45S</LockDuration>"));
    }

    #[test]
    fn unknown_elements_survive_an_update() {
        let mut d = full_queue();
        d.settings.max_delivery_count = Some(3);
        d.settings.forward_to = None;
        let written = queue_entry(&d);
        let names = element_names(&written, "QueueDescription");
        let at = |n: &str| names.iter().position(|x| x == n).unwrap();
        // In the fixture FutureFlag follows EnableBatchedOperations and precedes
        // IsAnonymousAccessible.
        assert!(
            written.contains("<FutureFlag>true</FutureFlag>"),
            "{written}"
        );
        assert_eq!(at("FutureFlag"), at("EnableBatchedOperations") + 1);
        assert!(at("FutureFlag") < at("IsAnonymousAccessible"));
        assert!(written.contains("<MaxDeliveryCount>3</MaxDeliveryCount>"));
        assert!(!names.iter().any(|n| n == "ForwardTo"), "cleared");
        let again = parse_queue(&xml::parse_exact(&written).unwrap()).unwrap();
        assert_eq!(again.raw.text_of("FutureFlag"), Some("true"));
    }

    #[test]
    fn new_entities_write_only_what_is_set() {
        let d = Description::new(QueueSettings {
            lock_duration: Some(Duration::from_secs(60)),
            ..Default::default()
        });
        let written = queue_entry(&d);
        assert_eq!(
            element_names(&written, "QueueDescription"),
            ["LockDuration", "AuthorizationRules"]
        );
        assert!(written.starts_with("<?xml"));
        assert!(written.contains("<content type=\"application/xml\">"));
    }

    #[test]
    fn topic_and_subscription_follow_their_order() {
        let entry = xml::parse_exact(include_str!("../../tests/fixtures/topic_entry.xml")).unwrap();
        let t = parse_topic(&entry).unwrap();
        assert_eq!(t.settings.support_ordering, Some(true));
        let written = topic_entry(&t);
        let names = element_names(&written, "TopicDescription");
        assert!(in_order(&names, TOPIC_ORDER), "{names:?}");
        assert!(
            names
                .iter()
                .any(|n| n == "FilteringMessagesBeforePublishing")
        );
        assert!(!names.iter().any(|n| n == "SubscriptionCount"));
        let again = parse_topic(&xml::parse_exact(&written).unwrap()).unwrap();
        assert_eq!(again.settings, t.settings);

        let entry =
            xml::parse_exact(include_str!("../../tests/fixtures/subscription_entry.xml")).unwrap();
        let s = parse_subscription(&entry).unwrap();
        assert_eq!(s.settings.max_delivery_count, Some(5));
        assert_eq!(
            s.settings.dead_lettering_on_filter_evaluation_exceptions,
            Some(true)
        );
        let rule = RuleSpec {
            name: "eu".into(),
            filter: RuleFilter::Sql {
                expression: "region = 'eu'".into(),
            },
            action: None,
        };
        let written = subscription_entry_with_rule(&s, Some(&Description::new(rule)));
        let names = element_names(&written, "SubscriptionDescription");
        assert!(in_order(&names, SUBSCRIPTION_ORDER), "{names:?}");
        assert!(names.iter().any(|n| n == "DefaultRuleDescription"));
        let again = parse_subscription(&xml::parse_exact(&written).unwrap()).unwrap();
        assert_eq!(again.settings, s.settings);
        assert!(
            !subscription_entry(&s).contains("DefaultRuleDescription"),
            "an update never sends the read-only default rule"
        );
    }

    #[test]
    fn sql_and_correlation_filters_write_their_i_type() {
        let entry = xml::parse_exact(include_str!("../../tests/fixtures/rule_entry.xml")).unwrap();
        let d = parse_rule(&entry).unwrap();
        assert_eq!(d.settings.name, "priority");
        let RuleFilter::Correlation { properties, .. } = &d.settings.filter else {
            panic!("{:?}", d.settings.filter)
        };
        assert_eq!(properties.len(), 2);
        let written = rule_entry(&d);
        assert!(
            written.contains("<Filter i:type=\"CorrelationFilter\">"),
            "{written}"
        );
        assert!(written.contains(
            "<KeyValueOfstringanyType><Key>region</Key><Value i:type=\"d6p1:string\" xmlns:d6p1=\"http://www.w3.org/2001/XMLSchema\">eu</Value></KeyValueOfstringanyType>"
        ), "{written}");
        assert!(
            written.contains(
                "i:type=\"d6p1:int\" xmlns:d6p1=\"http://www.w3.org/2001/XMLSchema\">2</Value>"
            ),
            "a property read as int stays an int: {written}"
        );
        assert!(written.contains(
            "<Action i:type=\"SqlRuleAction\"><SqlExpression>SET tier = 3</SqlExpression></Action>"
        ));
        let names = element_names(&written, "RuleDescription");
        assert_eq!(names, ["Filter", "Action", "Name"]);
        let again = parse_rule(&xml::parse_exact(&written).unwrap()).unwrap();
        assert_eq!(again.settings, d.settings);

        let mut sql = d.clone();
        sql.settings.filter = RuleFilter::Sql {
            expression: "a > 1 AND b < 2".into(),
        };
        sql.settings.action = None;
        let written = rule_entry(&sql);
        assert!(written.contains(
            "<Filter i:type=\"SqlFilter\"><SqlExpression>a &gt; 1 AND b &lt; 2</SqlExpression></Filter>"
        ), "{written}");
        assert!(!written.contains("<Action"));
        let t = rule_entry(&Description::new(RuleSpec {
            name: "$Default".into(),
            filter: RuleFilter::True,
            action: None,
        }));
        assert!(
            t.contains("<Filter i:type=\"TrueFilter\"><SqlExpression>1=1</SqlExpression></Filter>")
        );

        let new_prop = Description::new(RuleSpec {
            name: "c".into(),
            filter: RuleFilter::Correlation {
                correlation_id: None,
                message_id: None,
                to: None,
                reply_to: None,
                subject: Some("order.created".into()),
                session_id: None,
                reply_to_session_id: None,
                content_type: None,
                properties: vec![CorrelationProperty {
                    name: "tier".into(),
                    value: "2".into(),
                }],
            },
            action: None,
        });
        validate_filter(&new_prop.settings.filter).unwrap();
        let written = rule_entry(&new_prop);
        assert!(written.contains("<Label>order.created</Label><Properties>"));
        assert!(
            written.contains("i:type=\"d6p1:string\""),
            "a new property is a string"
        );
    }

    #[test]
    fn written_policies_carry_their_keys_and_rights() {
        let mut d = Description::new(QueueSettings::default());
        d.policies.push(AccessPolicy {
            key_name: "app".into(),
            rights: vec![AccessRight::Listen],
            primary_key: SecretString::from("p1"),
            secondary_key: SecretString::from("s1"),
        });
        let written = queue_entry(&d);
        assert!(written.contains(
            "<AuthorizationRule i:type=\"SharedAccessAuthorizationRule\"><ClaimType>SharedAccessKey</ClaimType><ClaimValue>None</ClaimValue><Rights><AccessRights>Listen</AccessRights></Rights><KeyName>app</KeyName><PrimaryKey>p1</PrimaryKey><SecondaryKey>s1</SecondaryKey></AuthorizationRule>"
        ), "{written}");
    }
}
