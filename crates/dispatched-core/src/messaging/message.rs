//! Display model for Service Bus messages and decoding from AMQP.

use crate::{Error, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fe2o3_amqp::types::messaging::annotations::AnnotationKey;
use fe2o3_amqp::types::messaging::message::__private::Deserializable;
use fe2o3_amqp::types::messaging::{Body, Message, MessageId};
use fe2o3_amqp::types::primitives::{SimpleValue, Timestamp, Value};
use serde::{Serialize, Serializer};
use serde_amqp::descriptor::Descriptor;
use std::collections::BTreeMap;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Broker-side state of a message (`x-opt-message-state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageState {
    /// Available for receipt.
    Active,
    /// Set aside by a receiver; only receivable by sequence number.
    Deferred,
    /// Not yet enqueued (scheduled for later).
    Scheduled,
}

impl MessageState {
    /// Lowercase name, as used in filters and JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            MessageState::Active => "active",
            MessageState::Deferred => "deferred",
            MessageState::Scheduled => "scheduled",
        }
    }
}

/// A message as shown to users and agents (peeked; no lock held).
#[derive(Debug, Clone, Serialize)]
pub struct PeekedMessage {
    /// Broker-assigned sequence number.
    pub sequence_number: i64,
    /// Active, deferred or scheduled.
    pub state: MessageState,
    /// `MessageId`.
    pub message_id: Option<String>,
    /// `CorrelationId`.
    pub correlation_id: Option<String>,
    /// `SessionId` (AMQP group-id).
    pub session_id: Option<String>,
    /// `Subject` (formerly Label).
    pub subject: Option<String>,
    /// `ContentType`.
    pub content_type: Option<String>,
    /// `To`.
    pub to: Option<String>,
    /// `ReplyTo`.
    pub reply_to: Option<String>,
    /// When the broker accepted the message.
    #[serde(with = "time::serde::rfc3339::option")]
    pub enqueued_time: Option<OffsetDateTime>,
    /// Scheduled enqueue time, for scheduled messages.
    #[serde(with = "time::serde::rfc3339::option")]
    pub scheduled_enqueue_time: Option<OffsetDateTime>,
    /// Lock expiry, for locked messages.
    #[serde(with = "time::serde::rfc3339::option")]
    pub locked_until: Option<OffsetDateTime>,
    /// Delivery attempts so far.
    pub delivery_count: u32,
    /// Time-to-live in ms, if set on the message.
    pub time_to_live_ms: Option<u32>,
    /// `DeadLetterReason` application property.
    pub dead_letter_reason: Option<String>,
    /// `DeadLetterErrorDescription` application property.
    pub dead_letter_description: Option<String>,
    /// Entity the message was dead-lettered from (`x-opt-deadletter-source`).
    pub dead_letter_source: Option<String>,
    /// Custom (application) properties.
    pub application_properties: BTreeMap<String, PropValue>,
    /// Body.
    pub body: MessageBody,
}

/// Message body in display form.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum MessageBody {
    /// One or more data sections, concatenated.
    Binary(#[serde(serialize_with = "as_base64")] Vec<u8>),
    /// AMQP-value string body.
    Text(String),
    /// Other AMQP-value body, converted to JSON for display.
    Value(serde_json::Value),
    /// AMQP-sequence body (one JSON array per section), converted to JSON for display.
    Sequence(serde_json::Value),
    /// No body.
    Empty,
}

fn as_base64<S: Serializer>(bytes: &[u8], s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&STANDARD.encode(bytes))
}

impl MessageBody {
    /// Body as text, if it is (valid UTF-8) text.
    pub fn as_text(&self) -> Option<String> {
        match self {
            MessageBody::Binary(b) => String::from_utf8(b.clone()).ok(),
            MessageBody::Text(s) => Some(s.clone()),
            MessageBody::Value(j) | MessageBody::Sequence(j) => Some(j.to_string()),
            MessageBody::Empty => None,
        }
    }

    /// Size in bytes (UTF-8 length for text forms; compact JSON length for value/sequence).
    pub fn len(&self) -> usize {
        match self {
            MessageBody::Binary(b) => b.len(),
            MessageBody::Text(s) => s.len(),
            MessageBody::Value(j) | MessageBody::Sequence(j) => j.to_string().len(),
            MessageBody::Empty => 0,
        }
    }

    /// Whether the body is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A typed application-property value.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum PropValue {
    /// null
    Null,
    /// bool
    Bool(bool),
    /// any signed integer
    Int(i64),
    /// any unsigned integer
    UInt(u64),
    /// float / double
    Double(f64),
    /// string or symbol
    String(String),
    /// timestamp
    #[serde(with = "time::serde::rfc3339")]
    Timestamp(OffsetDateTime),
    /// uuid
    Uuid(String),
    /// binary
    Binary(#[serde(serialize_with = "as_base64")] Vec<u8>),
    /// anything else, rendered
    Other(String),
}

impl PropValue {
    /// String content, if this is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            PropValue::String(s) => Some(s),
            _ => None,
        }
    }
}

/// Convert an AMQP timestamp without panicking; `None` when the (sender-controlled)
/// value falls outside years 0..=9999, which RFC 3339 can represent.
fn timestamp_to_datetime(ts: &Timestamp) -> Option<OffsetDateTime> {
    let nanos = i128::from(ts.milliseconds()) * 1_000_000;
    let dt = OffsetDateTime::from_unix_timestamp_nanos(nanos).ok()?;
    (0..=9999).contains(&dt.year()).then_some(dt)
}

/// Render an AMQP value as JSON for display. Never panics.
fn amqp_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    fn float(f: f64) -> J {
        serde_json::Number::from_f64(f).map_or_else(|| J::String(f.to_string()), J::Number)
    }
    match v {
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Ubyte(n) => J::from(*n),
        Value::Ushort(n) => J::from(*n),
        Value::Uint(n) => J::from(*n),
        Value::Ulong(n) => J::from(*n),
        Value::Byte(n) => J::from(*n),
        Value::Short(n) => J::from(*n),
        Value::Int(n) => J::from(*n),
        Value::Long(n) => J::from(*n),
        Value::Float(f) => float(f64::from(f.into_inner())),
        Value::Double(f) => float(f.into_inner()),
        Value::String(s) => J::String(s.clone()),
        Value::Symbol(s) => J::String(s.0.clone()),
        Value::Binary(b) => J::String(STANDARD.encode(b.as_slice())),
        Value::Timestamp(t) => {
            match timestamp_to_datetime(t).and_then(|dt| dt.format(&Rfc3339).ok()) {
                Some(s) => J::String(s),
                None => J::from(t.milliseconds()),
            }
        }
        Value::Uuid(u) => J::String(uuid::Uuid::from(u.clone()).to_string()),
        Value::List(items) => J::Array(items.iter().map(amqp_to_json).collect()),
        Value::Array(items) => J::Array(items.0.iter().map(amqp_to_json).collect()),
        Value::Map(map) => J::Object(
            map.iter()
                .map(|(k, val)| {
                    let key = match k {
                        Value::String(s) => s.clone(),
                        Value::Symbol(s) => s.0.clone(),
                        other => amqp_to_json(other).to_string(),
                    };
                    (key, amqp_to_json(val))
                })
                .collect(),
        ),
        Value::Described(d) => {
            let descriptor = match &d.descriptor {
                Descriptor::Name(s) => J::String(s.0.clone()),
                Descriptor::Code(c) => J::from(*c),
            };
            serde_json::json!({ "descriptor": descriptor, "value": amqp_to_json(&d.value) })
        }
        other => J::String(format!("{other:?}")),
    }
}

fn simple_to_prop(v: &SimpleValue) -> PropValue {
    match v {
        SimpleValue::Null => PropValue::Null,
        SimpleValue::Bool(b) => PropValue::Bool(*b),
        SimpleValue::Byte(n) => PropValue::Int(i64::from(*n)),
        SimpleValue::Short(n) => PropValue::Int(i64::from(*n)),
        SimpleValue::Int(n) => PropValue::Int(i64::from(*n)),
        SimpleValue::Long(n) => PropValue::Int(*n),
        SimpleValue::Ubyte(n) => PropValue::UInt(u64::from(*n)),
        SimpleValue::Ushort(n) => PropValue::UInt(u64::from(*n)),
        SimpleValue::Uint(n) => PropValue::UInt(u64::from(*n)),
        SimpleValue::Ulong(n) => PropValue::UInt(*n),
        SimpleValue::Float(f) => PropValue::Double(f64::from(f.into_inner())),
        SimpleValue::Double(f) => PropValue::Double(f.into_inner()),
        SimpleValue::String(s) => PropValue::String(s.clone()),
        SimpleValue::Symbol(s) => PropValue::String(s.0.clone()),
        SimpleValue::Timestamp(t) => match timestamp_to_datetime(t) {
            Some(dt) => PropValue::Timestamp(dt),
            None => PropValue::Int(t.milliseconds()),
        },
        SimpleValue::Uuid(u) => PropValue::Uuid(uuid::Uuid::from(u.clone()).to_string()),
        SimpleValue::Binary(b) => PropValue::Binary(b.to_vec()),
        other => PropValue::Other(format!("{other:?}")),
    }
}

fn message_id_string(id: &MessageId) -> String {
    match id {
        MessageId::String(s) => s.clone(),
        MessageId::Ulong(n) => n.to_string(),
        MessageId::Uuid(u) => uuid::Uuid::from(u.clone()).to_string(),
        MessageId::Binary(b) => b.iter().map(|byte| format!("{byte:02x}")).collect(),
    }
}

fn annotation<'a>(m: &'a Message<Body<Value>>, key: &str) -> Option<&'a Value> {
    m.message_annotations
        .as_ref()
        .and_then(|ma| ma.get(&key as &dyn AnnotationKey))
}

fn value_as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Byte(n) => Some(i64::from(*n)),
        Value::Short(n) => Some(i64::from(*n)),
        Value::Int(n) => Some(i64::from(*n)),
        Value::Long(n) => Some(*n),
        Value::Ubyte(n) => Some(i64::from(*n)),
        Value::Ushort(n) => Some(i64::from(*n)),
        Value::Uint(n) => Some(i64::from(*n)),
        Value::Ulong(n) => i64::try_from(*n).ok(),
        _ => None,
    }
}

fn annotation_time(m: &Message<Body<Value>>, key: &str) -> Option<OffsetDateTime> {
    match annotation(m, key)? {
        Value::Timestamp(t) => timestamp_to_datetime(t),
        _ => None,
    }
}

fn annotation_string(m: &Message<Body<Value>>, key: &str) -> Option<String> {
    match annotation(m, key)? {
        Value::String(s) => Some(s.clone()),
        Value::Symbol(s) => Some(s.0.clone()),
        _ => None,
    }
}

/// Convert an AMQP message into the display model.
pub fn decode_message(m: Message<Body<Value>>) -> PeekedMessage {
    let sequence_number = match annotation(&m, "x-opt-sequence-number") {
        Some(Value::Long(n)) => *n,
        _ => -1,
    };
    let state = match annotation(&m, "x-opt-message-state").and_then(value_as_i64) {
        Some(1) => MessageState::Deferred,
        Some(2) => MessageState::Scheduled,
        _ => MessageState::Active,
    };
    let props = m.properties.as_ref();
    let header = m.header.as_ref();
    let mut application_properties = BTreeMap::new();
    if let Some(ap) = m.application_properties.as_ref() {
        for (k, v) in ap.0.iter() {
            application_properties.insert(k.clone(), simple_to_prop(v));
        }
    }
    let dead_letter_reason = application_properties
        .get("DeadLetterReason")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let dead_letter_description = application_properties
        .get("DeadLetterErrorDescription")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let body = match &m.body {
        Body::Data(batch) => {
            let bytes: Vec<u8> = batch.iter().flat_map(|d| d.0.iter().copied()).collect();
            if bytes.is_empty() {
                MessageBody::Empty
            } else {
                MessageBody::Binary(bytes)
            }
        }
        Body::Value(v) => match &v.0 {
            Value::String(s) => MessageBody::Text(s.clone()),
            Value::Symbol(s) => MessageBody::Text(s.0.clone()),
            Value::Binary(b) => MessageBody::Binary(b.to_vec()),
            Value::Null => MessageBody::Empty,
            other => MessageBody::Value(amqp_to_json(other)),
        },
        Body::Sequence(seq) => MessageBody::Sequence(serde_json::Value::Array(
            seq.iter()
                .map(|section| {
                    serde_json::Value::Array(section.0.iter().map(amqp_to_json).collect())
                })
                .collect(),
        )),
        Body::Empty => MessageBody::Empty,
    };
    PeekedMessage {
        sequence_number,
        state,
        message_id: props
            .and_then(|p| p.message_id.as_ref())
            .map(message_id_string),
        correlation_id: props
            .and_then(|p| p.correlation_id.as_ref())
            .map(message_id_string),
        session_id: props.and_then(|p| p.group_id.clone()),
        subject: props.and_then(|p| p.subject.clone()),
        content_type: props
            .and_then(|p| p.content_type.as_ref())
            .map(|s| s.0.clone()),
        to: props.and_then(|p| p.to.clone()),
        reply_to: props.and_then(|p| p.reply_to.clone()),
        enqueued_time: annotation_time(&m, "x-opt-enqueued-time"),
        scheduled_enqueue_time: annotation_time(&m, "x-opt-scheduled-enqueue-time"),
        locked_until: annotation_time(&m, "x-opt-locked-until"),
        delivery_count: header.map(|h| h.delivery_count).unwrap_or(0),
        time_to_live_ms: header.and_then(|h| h.ttl),
        dead_letter_reason,
        dead_letter_description,
        dead_letter_source: annotation_string(&m, "x-opt-deadletter-source"),
        application_properties,
        body,
    }
}

/// Decode a binary-encoded AMQP message (as returned by peek / receive-by-sequence-number).
pub fn decode_bytes(bytes: &[u8]) -> Result<PeekedMessage> {
    let decoded: Deserializable<Message<Body<Value>>> = serde_amqp::from_slice(bytes)
        .map_err(|e| Error::Protocol(format!("cannot decode AMQP message: {e}")))?;
    Ok(decode_message(decoded.0))
}

#[cfg(test)]
// Shared fixture for later M1a tests; no caller yet.
#[allow(dead_code)]
pub(crate) fn test_message(seq: i64) -> PeekedMessage {
    PeekedMessage {
        sequence_number: seq,
        state: MessageState::Active,
        message_id: None,
        correlation_id: None,
        session_id: None,
        subject: None,
        content_type: None,
        to: None,
        reply_to: None,
        enqueued_time: None,
        scheduled_enqueue_time: None,
        locked_until: None,
        delivery_count: 0,
        time_to_live_ms: None,
        dead_letter_reason: None,
        dead_letter_description: None,
        dead_letter_source: None,
        application_properties: BTreeMap::new(),
        body: MessageBody::Text(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fe2o3_amqp::types::messaging::message::__private::Serializable;
    use fe2o3_amqp::types::messaging::{
        AmqpSequence, AmqpValue, ApplicationProperties, Batch, Data, Header, MessageAnnotations,
        Properties,
    };
    use fe2o3_amqp::types::primitives::OrderedMap;

    fn base(body: Body<Value>) -> Message<Body<Value>> {
        let mut app = OrderedMap::new();
        app.insert(
            "DeadLetterReason".to_string(),
            SimpleValue::String("MaxDeliveryCountExceeded".into()),
        );
        app.insert("tenant".to_string(), SimpleValue::Int(7));
        Message {
            header: Some(Header {
                delivery_count: 3,
                ..Default::default()
            }),
            delivery_annotations: None,
            message_annotations: Some(
                MessageAnnotations::builder()
                    .insert("x-opt-sequence-number", 42i64)
                    .build(),
            ),
            properties: Some(
                Properties::builder()
                    .message_id(String::from("m-1"))
                    .subject("order.created")
                    .content_type("application/json")
                    .group_id(Some("s-1".to_string()))
                    .build(),
            ),
            application_properties: Some(ApplicationProperties(app)),
            body,
            footer: None,
        }
    }

    fn with_state(state: Option<i32>) -> Message<Body<Value>> {
        let mut ann = MessageAnnotations::builder().insert("x-opt-sequence-number", 5i64);
        if let Some(s) = state {
            ann = ann.insert("x-opt-message-state", s);
        }
        Message {
            header: None,
            delivery_annotations: None,
            message_annotations: Some(ann.build()),
            properties: None,
            application_properties: None,
            body: Body::Data(Batch::from(vec![Data::from(b"x".to_vec())])),
            footer: None,
        }
    }

    #[test]
    fn message_state_comes_from_annotation() {
        assert_eq!(decode_message(with_state(None)).state, MessageState::Active);
        assert_eq!(
            decode_message(with_state(Some(0))).state,
            MessageState::Active
        );
        assert_eq!(
            decode_message(with_state(Some(1))).state,
            MessageState::Deferred
        );
        assert_eq!(
            decode_message(with_state(Some(2))).state,
            MessageState::Scheduled
        );
        assert_eq!(
            decode_message(with_state(Some(9))).state,
            MessageState::Active
        );
    }

    #[test]
    fn message_state_serialises_lowercase() {
        let json = serde_json::to_value(decode_message(with_state(Some(2)))).unwrap();
        assert_eq!(json["state"], "scheduled");
    }

    #[test]
    fn decodes_system_and_custom_properties() {
        let m = decode_message(base(Body::Data(Batch::from(vec![Data::from(
            b"{\"a\":1}".to_vec(),
        )]))));
        assert_eq!(m.sequence_number, 42);
        assert_eq!(m.message_id.as_deref(), Some("m-1"));
        assert_eq!(m.session_id.as_deref(), Some("s-1"));
        assert_eq!(m.subject.as_deref(), Some("order.created"));
        assert_eq!(m.content_type.as_deref(), Some("application/json"));
        assert_eq!(m.delivery_count, 3);
        assert_eq!(
            m.dead_letter_reason.as_deref(),
            Some("MaxDeliveryCountExceeded")
        );
        assert_eq!(m.application_properties["tenant"], PropValue::Int(7));
        assert_eq!(m.body.as_text().as_deref(), Some("{\"a\":1}"));
    }

    #[test]
    fn decodes_amqp_value_string_body() {
        let m = decode_message(base(Body::Value(AmqpValue(Value::String("hello".into())))));
        assert_eq!(m.body, MessageBody::Text("hello".into()));
    }

    #[test]
    fn decodes_multiple_data_sections() {
        let body = Body::Data(Batch::from(vec![
            Data::from(b"ab".to_vec()),
            Data::from(b"cd".to_vec()),
        ]));
        assert_eq!(
            decode_message(base(body)).body,
            MessageBody::Binary(b"abcd".to_vec())
        );
    }

    #[test]
    fn non_utf8_binary_has_no_text() {
        let m = decode_message(base(Body::Data(Batch::from(vec![Data::from(vec![
            0xff, 0xfe, 0x00,
        ])]))));
        assert_eq!(m.body.as_text(), None);
        assert_eq!(m.body.len(), 3);
    }

    #[test]
    fn empty_body_and_missing_annotations_do_not_panic() {
        let mut msg = base(Body::Empty);
        msg.message_annotations = None;
        msg.header = None;
        let m = decode_message(msg);
        assert_eq!(m.body, MessageBody::Empty);
        assert_eq!(m.sequence_number, -1);
        assert_eq!(m.delivery_count, 0);
    }

    #[test]
    fn roundtrips_through_wire_bytes() {
        let msg = Message::builder()
            .properties(
                Properties::builder()
                    .message_id(String::from("wire"))
                    .build(),
            )
            .message_annotations(
                MessageAnnotations::builder()
                    .insert("x-opt-sequence-number", 7i64)
                    .build(),
            )
            .data(b"payload".to_vec())
            .build();
        let bytes = serde_amqp::to_vec(&Serializable(msg)).unwrap();
        let m = decode_bytes(&bytes).unwrap();
        assert_eq!(m.sequence_number, 7);
        assert_eq!(m.message_id.as_deref(), Some("wire"));
        assert_eq!(m.body.as_text().as_deref(), Some("payload"));
    }

    #[test]
    fn json_shape_is_stable() {
        let m = decode_message(base(Body::Data(Batch::from(vec![Data::from(vec![
            1, 2, 3,
        ])]))));
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["body"]["kind"], "binary");
        assert_eq!(json["body"]["value"], "AQID");
        assert_eq!(json["application_properties"]["tenant"]["type"], "int");
    }

    #[test]
    fn non_string_value_and_sequence_bodies_render_as_json() {
        let v = decode_message(base(Body::Value(AmqpValue(Value::Long(5)))));
        assert_eq!(v.body, MessageBody::Value(serde_json::json!(5)));
        assert_eq!(v.body.as_text().as_deref(), Some("5"));
        assert_eq!(v.body.len(), 1);

        let mut map = OrderedMap::new();
        map.insert(
            Value::String("k".into()),
            Value::List(vec![Value::Bool(true), Value::Null]),
        );
        map.insert(Value::Int(2), Value::Double(f64::NAN.into()));
        map.insert(
            Value::Symbol("sym".into()),
            Value::Binary(vec![1, 2, 3].into()),
        );
        let m = decode_message(base(Body::Value(AmqpValue(Value::Map(map)))));
        assert_eq!(
            m.body,
            MessageBody::Value(serde_json::json!({"k": [true, null], "2": "NaN", "sym": "AQID"}))
        );

        let s = decode_message(base(Body::Sequence(Batch::from(vec![AmqpSequence(vec![
            Value::Long(1),
            Value::String("x".into()),
        ])]))));
        assert_eq!(s.body, MessageBody::Sequence(serde_json::json!([[1, "x"]])));
        assert_eq!(s.body.as_text().as_deref(), Some(r#"[[1,"x"]]"#));

        let t = decode_message(base(Body::Value(AmqpValue(Value::Symbol("sy".into())))));
        assert_eq!(t.body, MessageBody::Text("sy".into()));
        let b = decode_message(base(Body::Value(AmqpValue(Value::Binary(
            vec![9, 8].into(),
        )))));
        assert_eq!(b.body, MessageBody::Binary(vec![9, 8]));
        let n = decode_message(base(Body::Value(AmqpValue(Value::Null))));
        assert_eq!(n.body, MessageBody::Empty);
    }

    #[test]
    fn empty_data_sections_are_an_empty_body() {
        let e = decode_message(base(Body::Data(Batch::from(Vec::<Data>::new()))));
        assert_eq!(e.body, MessageBody::Empty);
        let e = decode_message(base(Body::Data(Batch::from(vec![
            Data::from(Vec::new()),
            Data::from(Vec::new()),
        ]))));
        assert_eq!(e.body, MessageBody::Empty);
        assert_eq!(
            serde_json::to_value(&e).unwrap()["body"],
            serde_json::json!({"kind": "empty"})
        );
    }

    #[test]
    fn out_of_range_timestamps_do_not_panic() {
        let ts = |ms: i64| Timestamp::from(ms);
        let mut msg = base(Body::Value(AmqpValue(Value::Timestamp(ts(i64::MAX)))));
        msg.message_annotations = Some(
            MessageAnnotations::builder()
                .insert("x-opt-enqueued-time", ts(i64::MAX))
                .insert("x-opt-scheduled-enqueue-time", ts(i64::MIN))
                .insert("x-opt-locked-until", ts(300_000_000_000_000)) // year > 9999
                .build(),
        );
        let mut app = OrderedMap::new();
        app.insert("max".to_string(), SimpleValue::Timestamp(ts(i64::MAX)));
        app.insert("min".to_string(), SimpleValue::Timestamp(ts(i64::MIN)));
        app.insert(
            "far".to_string(),
            SimpleValue::Timestamp(ts(300_000_000_000_000)),
        );
        app.insert(
            "ok".to_string(),
            SimpleValue::Timestamp(ts(1_700_000_000_000)),
        );
        msg.application_properties = Some(ApplicationProperties(app));
        let m = decode_message(msg);
        assert_eq!(m.enqueued_time, None);
        assert_eq!(m.scheduled_enqueue_time, None);
        assert_eq!(m.locked_until, None);
        assert_eq!(m.application_properties["max"], PropValue::Int(i64::MAX));
        assert_eq!(m.application_properties["min"], PropValue::Int(i64::MIN));
        assert_eq!(
            m.application_properties["far"],
            PropValue::Int(300_000_000_000_000)
        );
        assert!(matches!(
            m.application_properties["ok"],
            PropValue::Timestamp(_)
        ));
        assert_eq!(m.body, MessageBody::Value(serde_json::json!(i64::MAX)));
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(
            json["application_properties"]["ok"]["value"],
            "2023-11-14T22:13:20Z"
        );
        assert!(json["enqueued_time"].is_null());
    }

    #[test]
    fn wire_roundtrip_extracts_every_field() {
        let enq = 1_700_000_000_000_i64;
        let ts = |ms: i64| Timestamp::from(ms);
        let uuid = uuid::Uuid::parse_str("67e55044-10b1-426f-9247-bb680e5fe0c8").unwrap();
        let mut app = OrderedMap::new();
        app.insert(
            "DeadLetterReason".to_string(),
            SimpleValue::String("why".into()),
        );
        app.insert(
            "DeadLetterErrorDescription".to_string(),
            SimpleValue::String("desc".into()),
        );
        app.insert("when".to_string(), SimpleValue::Timestamp(ts(enq)));
        app.insert("id".to_string(), SimpleValue::Uuid(uuid.into()));
        app.insert(
            "blob".to_string(),
            SimpleValue::Binary(vec![1, 2, 3].into()),
        );
        app.insert("nothing".to_string(), SimpleValue::Null);
        let build = |id: MessageId| Message::<Body<Value>> {
            header: Some(Header {
                delivery_count: 4,
                ttl: Some(60_000),
                ..Default::default()
            }),
            delivery_annotations: None,
            message_annotations: Some(
                MessageAnnotations::builder()
                    .insert("x-opt-sequence-number", 99i64)
                    .insert("x-opt-enqueued-time", ts(enq))
                    .insert("x-opt-scheduled-enqueue-time", ts(enq + 1000))
                    .insert("x-opt-locked-until", ts(enq + 2000))
                    .insert("x-opt-deadletter-source", "orders")
                    .build(),
            ),
            properties: Some(
                Properties::builder()
                    .message_id(id)
                    .correlation_id(String::from("corr-1"))
                    .group_id(Some("sess-1".to_string()))
                    .subject("subj")
                    .content_type("text/plain")
                    .to("to-addr")
                    .reply_to("reply-addr")
                    .build(),
            ),
            application_properties: Some(ApplicationProperties(app.clone())),
            body: Body::Data(Batch::from(vec![Data::from(b"hi".to_vec())])),
            footer: None,
        };
        let decode = |id| {
            let bytes = serde_amqp::to_vec(&Serializable(build(id))).unwrap();
            decode_bytes(&bytes).unwrap()
        };

        let m = decode(MessageId::Ulong(12345));
        assert_eq!(m.message_id.as_deref(), Some("12345"));
        assert_eq!(m.sequence_number, 99);
        assert_eq!(m.correlation_id.as_deref(), Some("corr-1"));
        assert_eq!(m.session_id.as_deref(), Some("sess-1"));
        assert_eq!(m.subject.as_deref(), Some("subj"));
        assert_eq!(m.content_type.as_deref(), Some("text/plain"));
        assert_eq!(m.to.as_deref(), Some("to-addr"));
        assert_eq!(m.reply_to.as_deref(), Some("reply-addr"));
        assert_eq!(m.time_to_live_ms, Some(60_000));
        assert_eq!(m.delivery_count, 4);
        let at = |ms: i64| OffsetDateTime::from_unix_timestamp(ms / 1000).unwrap();
        assert_eq!(m.enqueued_time, Some(at(enq)));
        assert_eq!(m.scheduled_enqueue_time, Some(at(enq + 1000)));
        assert_eq!(m.locked_until, Some(at(enq + 2000)));
        assert_eq!(m.dead_letter_source.as_deref(), Some("orders"));
        assert_eq!(m.dead_letter_reason.as_deref(), Some("why"));
        assert_eq!(m.dead_letter_description.as_deref(), Some("desc"));
        assert_eq!(
            m.application_properties["when"],
            PropValue::Timestamp(at(enq))
        );
        assert_eq!(
            m.application_properties["id"],
            PropValue::Uuid(uuid.to_string())
        );
        assert_eq!(
            m.application_properties["blob"],
            PropValue::Binary(vec![1, 2, 3])
        );
        assert_eq!(m.application_properties["nothing"], PropValue::Null);
        assert_eq!(m.body.as_text().as_deref(), Some("hi"));

        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(
            json["application_properties"]["nothing"],
            serde_json::json!({"type": "null"})
        );
        assert_eq!(
            json["application_properties"]["when"]["value"],
            "2023-11-14T22:13:20Z"
        );
        assert_eq!(json["enqueued_time"], "2023-11-14T22:13:20Z");

        let m = decode(MessageId::Uuid(uuid.into()));
        assert_eq!(m.message_id.as_deref(), Some(uuid.to_string().as_str()));
    }
}
