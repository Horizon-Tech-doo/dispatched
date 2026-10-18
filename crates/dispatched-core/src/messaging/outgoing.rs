//! The outgoing message: composer core, AMQP encoding and clone rules.

use super::message::{MessageBody, PeekedMessage, PropValue};
use super::props::TypedValue;
use crate::{Error, Result};
use fe2o3_amqp::types::messaging::{
    AmqpValue, ApplicationProperties, Body, Data, Header, Message, MessageAnnotations, MessageId,
    Properties,
};
use fe2o3_amqp::types::primitives::{OrderedMap, Timestamp, Value};
use serde_amqp::primitives::Symbol;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;
use time::OffsetDateTime;

/// The longest MessageId, SessionId, PartitionKey and ReplyToSessionId the broker accepts.
const MAX_ID_CHARS: usize = 128;

/// The body of a message to send.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum OutgoingBody {
    /// One Data section.
    Bytes(Vec<u8>),
    /// An AMQP-value string.
    AmqpString(String),
    /// A JSON document re-encoded as an AMQP-value (null, bool, long, double, string, list, map).
    AmqpJson(serde_json::Value),
    /// No content (sent as an empty Data section; the broker refuses a message with no body).
    #[default]
    Empty,
}

/// A message about to be sent, with every system property the broker understands.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OutgoingMessage {
    /// The body.
    pub body: OutgoingBody,
    /// `MessageId`.
    pub message_id: Option<String>,
    /// `SessionId`.
    pub session_id: Option<String>,
    /// `CorrelationId`.
    pub correlation_id: Option<String>,
    /// `Subject`.
    pub subject: Option<String>,
    /// `To`.
    pub to: Option<String>,
    /// `ReplyTo`.
    pub reply_to: Option<String>,
    /// `ReplyToSessionId`.
    pub reply_to_session_id: Option<String>,
    /// `ContentType`.
    pub content_type: Option<String>,
    /// `PartitionKey`.
    pub partition_key: Option<String>,
    /// Time to live.
    pub time_to_live: Option<Duration>,
    /// Scheduled enqueue time.
    pub scheduled_enqueue_time: Option<OffsetDateTime>,
    /// Custom properties.
    pub properties: BTreeMap<String, TypedValue>,
}

fn check_id(name: &str, v: &Option<String>) -> Result<()> {
    if let Some(s) = v {
        if s.is_empty() {
            return Err(Error::Invalid(format!(
                "{name} is empty; leave it out instead"
            )));
        }
        if s.chars().count() > MAX_ID_CHARS {
            return Err(Error::Invalid(format!(
                "{name} is longer than {MAX_ID_CHARS} characters"
            )));
        }
    }
    Ok(())
}

// Used by the send operation (a later M2a task); tested here.
fn json_to_amqp(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::Long(i),
            None => Value::Double(n.as_f64().unwrap_or(f64::NAN).into()),
        },
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(a) => Value::List(a.iter().map(json_to_amqp).collect()),
        serde_json::Value::Object(o) => Value::Map(
            o.iter()
                .map(|(k, v)| (Value::String(k.clone()), json_to_amqp(v)))
                .collect(),
        ),
    }
}

// Used by the send operation (a later M2a task); tested here.
fn timestamp(t: OffsetDateTime) -> Timestamp {
    Timestamp::from(
        i64::try_from(t.unix_timestamp_nanos().div_euclid(1_000_000)).unwrap_or(i64::MAX),
    )
}

impl OutgoingMessage {
    /// A message with a text body (sent as bytes).
    pub fn text(body: &str) -> Self {
        OutgoingMessage {
            body: OutgoingBody::Bytes(body.as_bytes().to_vec()),
            ..Default::default()
        }
    }

    /// Check the broker's limits, naming the offending property.
    pub fn validate(&self) -> Result<()> {
        check_id("MessageId", &self.message_id)?;
        check_id("SessionId", &self.session_id)?;
        check_id("PartitionKey", &self.partition_key)?;
        check_id("ReplyToSessionId", &self.reply_to_session_id)?;
        if let (Some(s), Some(p)) = (&self.session_id, &self.partition_key)
            && s != p
        {
            return Err(Error::Invalid(format!(
                "PartitionKey must equal SessionId when both are set (SessionId '{s}', PartitionKey '{p}')"
            )));
        }
        if self.time_to_live == Some(Duration::ZERO) {
            return Err(Error::Invalid(
                "Time to live must be more than zero.".into(),
            ));
        }
        Ok(())
    }

    /// Encode as an AMQP message; `now` is the creation time.
    // Used by the send operation (a later M2a task); tested here.
    pub(crate) fn to_amqp(&self, now: OffsetDateTime) -> Result<Message<Body<Value>>> {
        self.validate()?;
        let mut props = Properties::builder();
        if let Some(id) = &self.message_id {
            props = props.message_id(MessageId::String(id.clone()));
        }
        if let Some(v) = &self.session_id {
            props = props.group_id(v.clone());
        }
        if let Some(v) = &self.reply_to_session_id {
            props = props.reply_to_group_id(v.clone());
        }
        if let Some(v) = &self.correlation_id {
            props = props.correlation_id(MessageId::String(v.clone()));
        }
        if let Some(v) = &self.subject {
            props = props.subject(v.clone());
        }
        if let Some(v) = &self.to {
            props = props.to(v.clone());
        }
        if let Some(v) = &self.reply_to {
            props = props.reply_to(v.clone());
        }
        if let Some(v) = &self.content_type {
            props = props.content_type(Symbol::from(v.as_str()));
        }
        let mut header = None;
        if let Some(ttl) = self.time_to_live {
            let ms = u32::try_from(ttl.as_millis()).unwrap_or(u32::MAX);
            header = Some(Header {
                ttl: Some(ms),
                ..Default::default()
            });
            // Saturating: a TTL past the last representable date expires at that date.
            let expiry = time::Duration::try_from(ttl)
                .ok()
                .and_then(|d| now.checked_add(d))
                .unwrap_or(OffsetDateTime::new_utc(time::Date::MAX, time::Time::MAX));
            props = props
                .creation_time(timestamp(now))
                .absolute_expiry_time(timestamp(expiry));
        }
        let mut ann = MessageAnnotations::builder();
        let mut has_ann = false;
        if let Some(t) = self.scheduled_enqueue_time {
            ann = ann.insert(
                "x-opt-scheduled-enqueue-time",
                Value::Timestamp(timestamp(t)),
            );
            has_ann = true;
        }
        if let Some(k) = &self.partition_key {
            ann = ann.insert("x-opt-partition-key", Value::String(k.clone()));
            has_ann = true;
        }
        let mut app = OrderedMap::new();
        for (k, v) in &self.properties {
            app.insert(k.clone(), v.to_simple());
        }
        let body = match &self.body {
            OutgoingBody::Bytes(b) => Body::Data(vec![Data(b.clone().into())].try_into().unwrap()),
            OutgoingBody::Empty => Body::Data(vec![Data(Vec::new().into())].try_into().unwrap()),
            OutgoingBody::AmqpString(s) => Body::Value(AmqpValue(Value::String(s.clone()))),
            OutgoingBody::AmqpJson(j) => Body::Value(AmqpValue(json_to_amqp(j))),
        };
        Ok(Message {
            header,
            delivery_annotations: None,
            message_annotations: has_ann.then(|| ann.build()),
            properties: Some(props.build()),
            application_properties: (!app.is_empty()).then_some(ApplicationProperties(app)),
            body,
            footer: None,
        })
    }

    /// A copy of a peeked message, ready to send again, with notes for anything that changed.
    pub fn from_peeked(m: &PeekedMessage, opts: &CloneOptions) -> Cloned {
        let mut notes = Vec::new();
        let body = match &m.body {
            MessageBody::Binary(b) => OutgoingBody::Bytes(b.clone()),
            MessageBody::Text(s) => OutgoingBody::AmqpString(s.clone()),
            MessageBody::Value(v) => {
                notes.push("body re-encoded from JSON".to_string());
                OutgoingBody::AmqpJson(v.clone())
            }
            MessageBody::Sequence(v) => {
                notes.push("body re-encoded from JSON".to_string());
                OutgoingBody::AmqpJson(v.clone())
            }
            MessageBody::Empty => OutgoingBody::Empty,
        };
        let mut lost = Vec::new();
        let properties = copy_properties(&m.application_properties, opts, &mut notes, &mut lost);
        let message_id = if opts.new_message_id {
            Some(uuid::Uuid::new_v4().to_string())
        } else {
            m.message_id.clone()
        };
        Cloned {
            message: OutgoingMessage {
                body,
                message_id,
                session_id: m.session_id.clone(),
                correlation_id: m.correlation_id.clone(),
                subject: m.subject.clone(),
                to: m.to.clone(),
                reply_to: m.reply_to.clone(),
                reply_to_session_id: m.reply_to_session_id.clone(),
                content_type: m.content_type.clone(),
                partition_key: m.partition_key.clone(),
                time_to_live: m.time_to_live,
                scheduled_enqueue_time: None,
                properties,
            },
            notes,
            lost,
        }
    }
}

/// How to clone a message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CloneOptions {
    /// Give the copy a fresh random MessageId.
    pub new_message_id: bool,
    /// Keep `DeadLetterReason` and `DeadLetterErrorDescription`.
    pub keep_dead_letter_properties: bool,
    /// The source is a dead-letter queue.
    pub from_dead_letter: bool,
}

impl CloneOptions {
    /// The one dead-letter rule: broker-set dead-letter properties are dropped from copies.
    pub fn drops_property(&self, name: &str) -> bool {
        self.from_dead_letter
            && !self.keep_dead_letter_properties
            && matches!(name, "DeadLetterReason" | "DeadLetterErrorDescription")
    }
}

/// The one property conversion for clone, import and templates. Every value is copied as it is
/// except `other` ones, which cannot be re-encoded: their names go into `lost` and a note says
/// so.
pub(crate) fn copy_properties(
    props: &BTreeMap<String, PropValue>,
    opts: &CloneOptions,
    notes: &mut Vec<String>,
    lost: &mut Vec<String>,
) -> BTreeMap<String, TypedValue> {
    let mut out = BTreeMap::new();
    for (name, value) in props {
        if opts.drops_property(name) {
            continue;
        }
        match TypedValue::from_prop(value) {
            Some(t) => {
                out.insert(name.clone(), t);
            }
            None => {
                notes.push(format!("property {name} (unsupported type) was not copied"));
                lost.push(name.clone());
            }
        }
    }
    out
}

/// A cloned message and what changed while cloning.
#[derive(Debug, Clone, PartialEq)]
pub struct Cloned {
    /// The copy.
    pub message: OutgoingMessage,
    /// Human-readable notes about differences from the source.
    pub notes: Vec<String>,
    /// Custom properties the copy lacks because their type cannot be copied (`other`).
    pub lost: Vec<String>,
}

/// Edits applied to an outgoing message; `Some(None)` clears a field.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Edits {
    /// Replace the body.
    pub body: Option<OutgoingBody>,
    /// Properties to set.
    pub set: BTreeMap<String, TypedValue>,
    /// Properties to remove.
    pub remove: Vec<String>,
    /// New MessageId.
    pub message_id: Option<String>,
    /// SessionId.
    pub session_id: Option<Option<String>>,
    /// ReplyToSessionId.
    pub reply_to_session_id: Option<Option<String>>,
    /// PartitionKey.
    pub partition_key: Option<Option<String>>,
    /// CorrelationId.
    pub correlation_id: Option<Option<String>>,
    /// Subject.
    pub subject: Option<Option<String>>,
    /// ContentType.
    pub content_type: Option<Option<String>>,
    /// To.
    pub to: Option<Option<String>>,
    /// ReplyTo.
    pub reply_to: Option<Option<String>>,
    /// Time to live.
    pub time_to_live: Option<Option<Duration>>,
}

impl Edits {
    /// Apply the edits to `m`.
    pub fn apply(&self, m: &mut OutgoingMessage) {
        if let Some(b) = &self.body {
            m.body = b.clone();
        }
        for name in &self.remove {
            m.properties.remove(name);
        }
        for (k, v) in &self.set {
            m.properties.insert(k.clone(), v.clone());
        }
        if let Some(id) = &self.message_id {
            m.message_id = Some(id.clone());
        }
        if let Some(new) = &self.session_id {
            if self.partition_key.is_none()
                && m.session_id.is_some()
                && m.partition_key == m.session_id
            {
                m.partition_key = new.clone();
            }
            m.session_id = new.clone();
        }
        if let Some(v) = &self.partition_key {
            m.partition_key = v.clone();
        }
        if let Some(v) = &self.reply_to_session_id {
            m.reply_to_session_id = v.clone();
        }
        if let Some(v) = &self.correlation_id {
            m.correlation_id = v.clone();
        }
        if let Some(v) = &self.subject {
            m.subject = v.clone();
        }
        if let Some(v) = &self.content_type {
            m.content_type = v.clone();
        }
        if let Some(v) = &self.to {
            m.to = v.clone();
        }
        if let Some(v) = &self.reply_to {
            m.reply_to = v.clone();
        }
        if let Some(v) = &self.time_to_live {
            m.time_to_live = *v;
        }
    }
}

/// Where each message's MessageId comes from when sending several.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageIdSource {
    /// Leave as is.
    Keep,
    /// The same id on every message.
    Fixed(String),
    /// A random id per message.
    Random,
    /// `base`, then base plus one, two, ...; trailing digits keep their zero padding.
    Increment {
        /// The first id.
        base: String,
    },
}

impl MessageIdSource {
    /// The id for message number `index` (from 0); `None` means leave the message's id alone.
    pub fn id_for(&self, index: u64) -> Option<String> {
        match self {
            MessageIdSource::Keep => None,
            MessageIdSource::Fixed(s) => Some(s.clone()),
            MessageIdSource::Random => Some(uuid::Uuid::new_v4().to_string()),
            MessageIdSource::Increment { base } => {
                let split = base
                    .char_indices()
                    .rev()
                    .take_while(|(_, c)| c.is_ascii_digit())
                    .last()
                    .map(|(i, _)| i);
                if let Some(i) = split
                    && let Ok(n) = base[i..].parse::<u128>()
                    && let Some(next) = n.checked_add(u128::from(index))
                {
                    let width = base.len() - i;
                    return Some(format!("{}{next:0width$}", &base[..i]));
                }
                Some(if index == 0 {
                    base.clone()
                } else {
                    format!("{base}-{index}")
                })
            }
        }
    }
}

/// Where each message's body comes from when sending several.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodySource {
    /// The same text on every message.
    Text(String),
    /// Files used round-robin.
    Files(Vec<PathBuf>),
}

impl BodySource {
    /// The body for message number `index` (from 0); files are read on use.
    pub fn body_for(&self, index: u64) -> Result<OutgoingBody> {
        match self {
            BodySource::Text(s) => Ok(OutgoingBody::Bytes(s.as_bytes().to_vec())),
            BodySource::Files(files) => {
                if files.is_empty() {
                    return Err(Error::Invalid("no body files given".into()));
                }
                let n = u64::try_from(files.len()).unwrap_or(u64::MAX);
                let path = &files[usize::try_from(index % n).unwrap_or(0)];
                std::fs::read(path)
                    .map(OutgoingBody::Bytes)
                    .map_err(|e| Error::Io(format!("cannot read {}: {e}", path.display())))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::message::test_message;
    use fe2o3_amqp::types::messaging::annotations::AnnotationKey;
    use time::macros::datetime;

    #[test]
    fn amqp_encoding_sets_every_system_property() {
        let now = datetime!(2026-10-17 12:00 UTC);
        let m = OutgoingMessage {
            message_id: Some("m1".into()),
            session_id: Some("s".into()),
            correlation_id: Some("c".into()),
            subject: Some("sub".into()),
            to: Some("to".into()),
            reply_to: Some("rt".into()),
            reply_to_session_id: Some("rs".into()),
            content_type: Some("application/json".into()),
            partition_key: Some("s".into()),
            time_to_live: Some(Duration::from_secs(60)),
            scheduled_enqueue_time: Some(now + time::Duration::hours(1)),
            properties: BTreeMap::from([("n".into(), TypedValue::Int(1))]),
            ..OutgoingMessage::text("{}")
        };
        let a = m.to_amqp(now).unwrap();
        let p = a.properties.as_ref().unwrap();
        assert_eq!(p.message_id, Some(MessageId::String("m1".into())));
        assert_eq!(p.group_id.as_deref(), Some("s"));
        assert_eq!(p.reply_to_group_id.as_deref(), Some("rs"));
        assert_eq!(p.subject.as_deref(), Some("sub"));
        assert_eq!(p.to.as_deref(), Some("to"));
        assert_eq!(p.reply_to.as_deref(), Some("rt"));
        assert_eq!(
            p.content_type.as_ref().map(|s| s.0.as_str()),
            Some("application/json")
        );
        assert_eq!(a.header.as_ref().unwrap().ttl, Some(60_000));
        assert_eq!(
            p.absolute_expiry_time.as_ref().unwrap().milliseconds()
                - p.creation_time.as_ref().unwrap().milliseconds(),
            60_000
        );
        let ann = a.message_annotations.as_ref().unwrap();
        assert!(
            ann.get(&"x-opt-scheduled-enqueue-time" as &dyn AnnotationKey)
                .is_some()
        );
        assert!(
            ann.get(&"x-opt-partition-key" as &dyn AnnotationKey)
                .is_some()
        );
        assert!(a.application_properties.is_some());
        assert!(matches!(a.body, Body::Data(_)));
    }

    #[test]
    fn a_ttl_longer_than_the_header_caps_the_header_but_keeps_the_expiry() {
        let now = datetime!(2026-10-17 12:00 UTC);
        let m = OutgoingMessage {
            time_to_live: Some(Duration::from_secs(100 * 86_400)),
            ..OutgoingMessage::text("x")
        };
        let a = m.to_amqp(now).unwrap();
        assert_eq!(a.header.unwrap().ttl, Some(u32::MAX));
        let p = a.properties.unwrap();
        assert_eq!(
            p.absolute_expiry_time.unwrap().milliseconds()
                - p.creation_time.unwrap().milliseconds(),
            100 * 86_400_000
        );
    }

    #[test]
    fn a_huge_ttl_saturates_the_expiry_instead_of_panicking() {
        let now = datetime!(2026-10-17 12:00 UTC);
        for ttl in [Duration::from_secs(u64::MAX / 2), Duration::MAX] {
            let m = OutgoingMessage {
                time_to_live: Some(ttl),
                ..OutgoingMessage::text("x")
            };
            let a = m.to_amqp(now).unwrap();
            assert_eq!(a.header.unwrap().ttl, Some(u32::MAX));
            assert!(a.properties.unwrap().absolute_expiry_time.is_some());
        }
    }

    #[test]
    fn a_zero_ttl_is_refused() {
        let m = OutgoingMessage {
            time_to_live: Some(Duration::ZERO),
            ..OutgoingMessage::text("x")
        };
        assert!(
            matches!(m.validate(), Err(Error::Invalid(e)) if e == "Time to live must be more than zero.")
        );
    }

    #[test]
    fn a_session_id_on_a_plain_message_adds_no_partition_key() {
        let mut m = OutgoingMessage::text("b");
        Edits {
            session_id: Some(Some("x".into())),
            ..Default::default()
        }
        .apply(&mut m);
        assert_eq!(
            (m.session_id.as_deref(), m.partition_key.as_deref()),
            (Some("x"), None)
        );
    }

    #[test]
    fn bodies_encode_by_kind() {
        let now = datetime!(2026-10-17 12:00 UTC);
        let enc = |b| {
            OutgoingMessage {
                body: b,
                ..Default::default()
            }
            .to_amqp(now)
            .unwrap()
            .body
        };
        assert!(matches!(enc(OutgoingBody::Empty), Body::Data(_)));
        assert!(matches!(
            enc(OutgoingBody::AmqpString("s".into())),
            Body::Value(v) if v.0 == Value::String("s".into())
        ));
        let Body::Value(v) = enc(OutgoingBody::AmqpJson(
            serde_json::json!({"a": [1, 2.5, null]}),
        )) else {
            panic!("not a value body")
        };
        let Value::Map(m) = v.0 else {
            panic!("not a map")
        };
        assert_eq!(
            m.get(&Value::String("a".into())),
            Some(&Value::List(vec![
                Value::Long(1),
                Value::Double(2.5.into()),
                Value::Null
            ]))
        );
    }

    #[test]
    fn validation_names_the_broker_limits() {
        let long = "x".repeat(129);
        for (m, needle) in [
            (
                OutgoingMessage {
                    message_id: Some(long.clone()),
                    ..OutgoingMessage::text("x")
                },
                "MessageId",
            ),
            (
                OutgoingMessage {
                    session_id: Some(long.clone()),
                    ..OutgoingMessage::text("x")
                },
                "SessionId",
            ),
            (
                OutgoingMessage {
                    partition_key: Some(long),
                    ..OutgoingMessage::text("x")
                },
                "PartitionKey",
            ),
            (
                OutgoingMessage {
                    session_id: Some("a".into()),
                    partition_key: Some("b".into()),
                    ..OutgoingMessage::text("x")
                },
                "PartitionKey must equal SessionId",
            ),
            (
                OutgoingMessage {
                    reply_to_session_id: Some(String::new()),
                    ..OutgoingMessage::text("x")
                },
                "leave it out",
            ),
        ] {
            assert!(
                matches!(m.validate(), Err(Error::Invalid(e)) if e.contains(needle)),
                "{needle}"
            );
        }
    }

    #[test]
    fn clone_keeps_user_properties_and_drops_broker_values() {
        let mut src = test_message(42);
        src.body = MessageBody::Binary(b"{\"a\":1}".to_vec());
        src.message_id = Some("orig".into());
        src.session_id = Some("s".into());
        src.time_to_live = Some(Duration::from_secs(30));
        src.scheduled_enqueue_time = Some(datetime!(2030-01-01 0:00 UTC));
        src.application_properties
            .insert("DeadLetterReason".into(), PropValue::String("x".into()));
        src.application_properties
            .insert("kind".into(), PropValue::String("order".into()));
        src.application_properties
            .insert("blob".into(), PropValue::Binary(vec![1]));
        let c = OutgoingMessage::from_peeked(
            &src,
            &CloneOptions {
                from_dead_letter: true,
                ..Default::default()
            },
        );
        assert_eq!(c.message.body, OutgoingBody::Bytes(b"{\"a\":1}".to_vec()));
        assert_eq!(c.message.message_id.as_deref(), Some("orig"));
        assert_eq!(c.message.session_id.as_deref(), Some("s"));
        assert_eq!(c.message.time_to_live, Some(Duration::from_secs(30)));
        assert_eq!(c.message.scheduled_enqueue_time, None);
        assert!(!c.message.properties.contains_key("DeadLetterReason"));
        assert_eq!(
            c.message.properties["kind"],
            TypedValue::String("order".into())
        );
        assert_eq!(
            c.message.properties["blob"],
            TypedValue::Binary(vec![1]),
            "binary is copied as it is"
        );
        assert!(c.notes.is_empty() && c.lost.is_empty(), "{:?}", c.notes);
        let mut odd = src.clone();
        odd.application_properties
            .insert("weird".into(), PropValue::Other("described".into()));
        let c = OutgoingMessage::from_peeked(&odd, &CloneOptions::default());
        assert_eq!(c.lost, ["weird"]);
        assert!(!c.message.properties.contains_key("weird"));
        assert!(
            c.notes
                .iter()
                .any(|n| n == "property weird (unsupported type) was not copied"),
            "{:?}",
            c.notes
        );
        let fresh = OutgoingMessage::from_peeked(
            &src,
            &CloneOptions {
                new_message_id: true,
                ..Default::default()
            },
        );
        assert_ne!(fresh.message.message_id.as_deref(), Some("orig"));
        let dl = CloneOptions {
            from_dead_letter: true,
            ..Default::default()
        };
        assert!(
            dl.drops_property("DeadLetterReason")
                && dl.drops_property("DeadLetterErrorDescription")
                && !dl.drops_property("kind")
        );
        assert!(
            !CloneOptions {
                keep_dead_letter_properties: true,
                ..dl
            }
            .drops_property("DeadLetterReason")
        );
    }

    #[test]
    fn json_value_bodies_are_re_encoded_and_say_so() {
        let mut src = test_message(1);
        src.body = MessageBody::Value(serde_json::json!({"a": [1, 2]}));
        let c = OutgoingMessage::from_peeked(&src, &CloneOptions::default());
        assert!(matches!(c.message.body, OutgoingBody::AmqpJson(_)));
        assert_eq!(c.notes, ["body re-encoded from JSON"]);
    }

    #[test]
    fn edits_set_clear_and_remove() {
        let mut m = OutgoingMessage {
            subject: Some("a".into()),
            properties: BTreeMap::from([("x".into(), TypedValue::Int(1))]),
            ..OutgoingMessage::text("b")
        };
        Edits {
            subject: Some(None),
            remove: vec!["x".into()],
            set: BTreeMap::from([("y".into(), TypedValue::Bool(true))]),
            body: Some(OutgoingBody::Bytes(b"new".to_vec())),
            ..Default::default()
        }
        .apply(&mut m);
        assert_eq!(m.subject, None);
        assert!(!m.properties.contains_key("x") && m.properties.contains_key("y"));
        assert_eq!(m.body, OutgoingBody::Bytes(b"new".to_vec()));
    }

    #[test]
    fn a_new_session_id_takes_the_partition_key_along() {
        let mut m = OutgoingMessage {
            session_id: Some("old".into()),
            partition_key: Some("old".into()),
            ..OutgoingMessage::text("b")
        };
        Edits {
            session_id: Some(Some("new".into())),
            ..Default::default()
        }
        .apply(&mut m);
        assert_eq!(
            (m.session_id.as_deref(), m.partition_key.as_deref()),
            (Some("new"), Some("new"))
        );
        m.validate().unwrap();
        Edits {
            session_id: Some(None),
            ..Default::default()
        }
        .apply(&mut m);
        assert_eq!(
            (m.session_id.as_deref(), m.partition_key.as_deref()),
            (None, None)
        );
        let mut p = OutgoingMessage {
            session_id: Some("s".into()),
            partition_key: Some("s".into()),
            ..OutgoingMessage::text("b")
        };
        Edits {
            session_id: Some(Some("t".into())),
            partition_key: Some(Some("t".into())),
            reply_to_session_id: Some(Some("r".into())),
            ..Default::default()
        }
        .apply(&mut p);
        assert_eq!(
            (p.partition_key.as_deref(), p.reply_to_session_id.as_deref()),
            (Some("t"), Some("r"))
        );
        let mut q = OutgoingMessage {
            partition_key: Some("k".into()),
            ..OutgoingMessage::text("b")
        };
        Edits {
            session_id: Some(Some("x".into())),
            ..Default::default()
        }
        .apply(&mut q);
        assert_eq!(
            q.partition_key.as_deref(),
            Some("k"),
            "an unrelated partition key is left alone (validate then reports it)"
        );
    }

    #[test]
    fn message_ids_increment_keeping_padding() {
        let inc = MessageIdSource::Increment {
            base: "order-0009".into(),
        };
        assert_eq!(
            [inc.id_for(0), inc.id_for(1)],
            [Some("order-0009".into()), Some("order-0010".into())]
        );
        let plain = MessageIdSource::Increment {
            base: "order".into(),
        };
        assert_eq!(
            [plain.id_for(0), plain.id_for(1)],
            [Some("order".into()), Some("order-1".into())]
        );
        assert_eq!(
            MessageIdSource::Increment { base: "99".into() }.id_for(1),
            Some("100".into())
        );
        assert_eq!(MessageIdSource::Keep.id_for(3), None);
    }

    #[test]
    fn files_are_used_round_robin() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::write(&a, "A").unwrap();
        std::fs::write(&b, "B").unwrap();
        let s = BodySource::Files(vec![a, b]);
        let bodies: Vec<_> = (0..3).map(|i| s.body_for(i).unwrap()).collect();
        assert_eq!(
            bodies,
            [
                OutgoingBody::Bytes(b"A".to_vec()),
                OutgoingBody::Bytes(b"B".to_vec()),
                OutgoingBody::Bytes(b"A".to_vec())
            ]
        );
    }
}
