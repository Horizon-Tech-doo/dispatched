//! Display model for Service Bus messages and decoding from AMQP.

use crate::{Error, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fe2o3_amqp::types::messaging::annotations::AnnotationKey;
use fe2o3_amqp::types::messaging::message::__private::Deserializable;
use fe2o3_amqp::types::messaging::{Body, Message, MessageId};
use fe2o3_amqp::types::primitives::{SimpleValue, Value};
use serde::{Serialize, Serializer};
use std::collections::BTreeMap;
use time::OffsetDateTime;

/// A message as shown to users and agents (peeked; no lock held).
#[derive(Debug, Clone, Serialize)]
pub struct PeekedMessage {
    /// Broker-assigned sequence number.
    pub sequence_number: i64,
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
    /// Other AMQP-value body, rendered for display.
    Value(String),
    /// AMQP-sequence body, rendered for display.
    Sequence(String),
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
            MessageBody::Text(s) | MessageBody::Value(s) | MessageBody::Sequence(s) => {
                Some(s.clone())
            }
            MessageBody::Empty => None,
        }
    }

    /// Size in bytes (UTF-8 length for text forms).
    pub fn len(&self) -> usize {
        match self {
            MessageBody::Binary(b) => b.len(),
            MessageBody::Text(s) | MessageBody::Value(s) | MessageBody::Sequence(s) => s.len(),
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
        SimpleValue::Timestamp(t) => PropValue::Timestamp(OffsetDateTime::from(t.clone())),
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

fn annotation_time(m: &Message<Body<Value>>, key: &str) -> Option<OffsetDateTime> {
    match annotation(m, key)? {
        Value::Timestamp(t) => Some(OffsetDateTime::from(t.clone())),
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
            MessageBody::Binary(batch.iter().flat_map(|d| d.0.iter().copied()).collect())
        }
        Body::Value(v) => match &v.0 {
            Value::String(s) => MessageBody::Text(s.clone()),
            Value::Null => MessageBody::Empty,
            other => MessageBody::Value(format!("{other:?}")),
        },
        Body::Sequence(seq) => MessageBody::Sequence(format!("{seq:?}")),
        Body::Empty => MessageBody::Empty,
    };
    PeekedMessage {
        sequence_number,
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
    fn non_string_value_and_sequence_bodies_display_without_panicking() {
        let v = decode_message(base(Body::Value(AmqpValue(Value::Long(5)))));
        assert!(matches!(v.body, MessageBody::Value(_)));
        assert!(v.body.as_text().is_some());
        let s = decode_message(base(Body::Sequence(Batch::from(vec![AmqpSequence(vec![
            Value::Long(1),
        ])]))));
        assert!(matches!(s.body, MessageBody::Sequence(_)));
        let e = decode_message(base(Body::Data(Batch::from(Vec::<Data>::new()))));
        assert!(e.body.is_empty());
    }
}
