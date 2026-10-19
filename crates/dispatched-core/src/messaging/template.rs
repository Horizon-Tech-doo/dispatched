//! BrokeredMessageTemplate files (the incumbent explorer's single-message JSON and XML format).

use super::outgoing::{OutgoingBody, OutgoingMessage};
use super::props::{PropType, TypedValue, parse_duration};
use crate::mgmt::xml;
use crate::{Error, Result};
use std::collections::BTreeMap;
use std::path::Path;
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, PrimitiveDateTime};

/// The largest template file, in bytes.
pub const MAX_TEMPLATE_BYTES: u64 = 1024 * 1024;
const TYPE_NAMES: &str = "String, Int32, Int64, Double, Boolean, DateTime, TimeSpan, Guid";

/// A template field set, shared by the JSON and XML readers.
#[derive(Default)]
struct Raw {
    message_id: Option<String>,
    session_id: Option<String>,
    correlation_id: Option<String>,
    content_type: Option<String>,
    label: Option<String>,
    partition_key: Option<String>,
    to: Option<String>,
    reply_to: Option<String>,
    reply_to_session_id: Option<String>,
    time_to_live: Option<String>,
    scheduled: Option<String>,
    body: Option<String>,
    properties: Vec<(String, String, String)>,
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.filter(|s| !s.is_empty())
}

/// Parse `yyyy-MM-ddTHH:mm:ss` (UTC) or RFC 3339.
pub(crate) fn parse_utc_time(text: &str) -> Option<OffsetDateTime> {
    let t = text.trim();
    if let Ok(v) = OffsetDateTime::parse(t, &Rfc3339) {
        return Some(v);
    }
    let fmt = time::format_description::parse_borrowed::<2>(
        "[year]-[month]-[day]T[hour]:[minute]:[second]",
    )
    .ok()?;
    PrimitiveDateTime::parse(t, &fmt)
        .ok()
        .map(PrimitiveDateTime::assume_utc)
}

impl Raw {
    fn into_message(self) -> Result<OutgoingMessage> {
        let time_to_live = match non_empty(self.time_to_live) {
            Some(t) => Some(parse_duration(&t).map_err(|e| {
                Error::Invalid(format!("template timeToLive: {e}; use [d.]hh:mm:ss"))
            })?),
            None => None,
        };
        let scheduled_enqueue_time = match non_empty(self.scheduled) {
            Some(t) => Some(parse_utc_time(&t).ok_or_else(|| {
                Error::Invalid(format!(
                    "template scheduledEnqueueTimeUtc '{t}' is not a time; use yyyy-MM-ddTHH:mm:ss (UTC) or RFC 3339"
                ))
            })?),
            None => None,
        };
        let mut properties = BTreeMap::new();
        for (key, ty, value) in self.properties {
            if key.is_empty() {
                return Err(Error::Invalid(
                    "template property has no key; every property needs a key".into(),
                ));
            }
            let ty = PropType::parse(&ty).map_err(|_| {
                Error::Invalid(format!(
                    "template property '{key}' has unknown type '{ty}'; use one of {TYPE_NAMES}"
                ))
            })?;
            let typed = match (ty, parse_utc_time(&value)) {
                (PropType::DateTime, Some(t)) => TypedValue::DateTime(t),
                _ => TypedValue::parse(ty, &value)
                    .map_err(|e| Error::Invalid(format!("template property '{key}': {e}")))?,
            };
            if properties.insert(key.clone(), typed).is_some() {
                return Err(Error::Invalid(format!(
                    "template property '{key}' appears twice; keep one"
                )));
            }
        }
        let message = OutgoingMessage {
            body: match non_empty(self.body) {
                Some(b) => OutgoingBody::Bytes(b.into_bytes()),
                None => OutgoingBody::Empty,
            },
            message_id: non_empty(self.message_id),
            session_id: non_empty(self.session_id),
            correlation_id: non_empty(self.correlation_id),
            subject: non_empty(self.label),
            to: non_empty(self.to),
            reply_to: non_empty(self.reply_to),
            reply_to_session_id: non_empty(self.reply_to_session_id),
            content_type: non_empty(self.content_type),
            partition_key: non_empty(self.partition_key),
            time_to_live,
            scheduled_enqueue_time,
            properties,
        };
        message.validate()?;
        Ok(message)
    }
}

fn json_text(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Read a template file: JSON when the first non-space byte is `{`, XML when it is `<`.
pub fn read_template(path: &Path) -> Result<OutgoingMessage> {
    use std::io::Read;
    let io = |e: std::io::Error| {
        Error::Io(format!(
            "cannot read template {}: {e}; check the path and permissions",
            path.display()
        ))
    };
    // Read through take() so a special file cannot slip past the cap.
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(io)?
        .take(MAX_TEMPLATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > MAX_TEMPLATE_BYTES {
        return Err(Error::Invalid(format!(
            "template {} is larger than 1 MiB; a template holds one message",
            path.display()
        )));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| Error::Invalid(format!("template {} is not UTF-8 text", path.display())))?;
    let text = text.trim_start_matches('\u{feff}');
    match text.trim_start().as_bytes().first() {
        Some(b'{') => parse_template_json(text),
        Some(b'<') => parse_template_xml(text),
        _ => Err(Error::Invalid(format!(
            "template {} is neither JSON (starts with {{) nor XML (starts with <)",
            path.display()
        ))),
    }
}

/// Parse a JSON template.
pub fn parse_template_json(text: &str) -> Result<OutgoingMessage> {
    let v: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| Error::Invalid(format!("template is not valid JSON: {e}")))?;
    let obj = v
        .as_object()
        .ok_or_else(|| Error::Invalid("template JSON must be an object".into()))?;
    let get = |k: &str| obj.get(k).and_then(json_text);
    let mut raw = Raw {
        message_id: get("messageId"),
        session_id: get("sessionId"),
        correlation_id: get("correlationId"),
        content_type: get("contentType"),
        label: get("label"),
        partition_key: get("partitionKey"),
        to: get("to"),
        reply_to: get("replyTo"),
        reply_to_session_id: get("replyToSessionId"),
        time_to_live: get("timeToLive"),
        scheduled: get("scheduledEnqueueTimeUtc"),
        body: get("body"),
        properties: Vec::new(),
    };
    match obj.get("properties") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::Array(items)) => {
            for (i, item) in items.iter().enumerate() {
                let field = |k: &str| item.get(k).and_then(json_text).unwrap_or_default();
                if !item.is_object() {
                    return Err(Error::Invalid(format!(
                        "template property {} must be an object with key, type and value",
                        i + 1
                    )));
                }
                raw.properties
                    .push((field("key"), field("type"), field("value")));
            }
        }
        Some(_) => {
            return Err(Error::Invalid(
                "template properties must be a list of {key, type, value}".into(),
            ));
        }
    }
    raw.into_message()
}

/// Parse an XML template (root `brokeredMessage`, matched by local name).
pub fn parse_template_xml(text: &str) -> Result<OutgoingMessage> {
    let root = xml::parse_exact(text).map_err(|e| Error::Invalid(format!("template: {e}")))?;
    if root.name != "brokeredMessage" {
        return Err(Error::Invalid(format!(
            "template XML root is '{}'; expected brokeredMessage",
            root.name
        )));
    }
    // Only the body and property values keep their exact text; other fields are trimmed.
    let get = |k: &str| root.child(k).map(|n| n.text.trim().to_string());
    let mut raw = Raw {
        message_id: get("messageId"),
        session_id: get("sessionId"),
        correlation_id: get("correlationId"),
        content_type: get("contentType"),
        label: get("label"),
        partition_key: get("partitionKey"),
        to: get("to"),
        reply_to: get("replyTo"),
        reply_to_session_id: get("replyToSessionId"),
        time_to_live: get("timeToLive"),
        scheduled: get("scheduledEnqueueTimeUtc"),
        body: root.child("body").map(|n| n.text.clone()),
        properties: Vec::new(),
    };
    if let Some(props) = root.child("properties") {
        for p in props.children_named("property") {
            let field = |k: &str| {
                p.child(k)
                    .map(|n| n.text.trim().to_string())
                    .unwrap_or_default()
            };
            let value = p.child("value").map(|n| n.text.clone()).unwrap_or_default();
            raw.properties.push((field("key"), field("type"), value));
        }
    }
    raw.into_message()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use time::macros::datetime;

    #[test]
    fn json_template_maps_every_field() {
        let t = parse_template_json(include_str!("../../tests/fixtures/template.json")).unwrap();
        assert_eq!(t.message_id.as_deref(), Some("MyMessageId"));
        assert_eq!(t.subject.as_deref(), Some("MyLabel"));
        assert_eq!(t.time_to_live, Some(Duration::from_secs(3600)));
        assert_eq!(t.properties["Country"], TypedValue::String("Italy".into()));
        assert_eq!(t.properties["Quantity"], TypedValue::Int(5));
        assert_eq!(t.body, OutgoingBody::Bytes(b"{\"orderId\":1}".to_vec()));
        assert_eq!(t.session_id.as_deref(), Some("MySession"));
        assert_eq!(t.partition_key.as_deref(), Some("MySession"));
        assert_eq!(t.reply_to_session_id.as_deref(), Some("MyReplySession"));
        assert_eq!(
            t.scheduled_enqueue_time,
            Some(datetime!(2026-10-17 9:30 UTC))
        );
    }

    #[test]
    fn xml_template_reads_the_incumbent_namespace_and_cdata_body() {
        let t = parse_template_xml(include_str!("../../tests/fixtures/template.xml")).unwrap();
        assert_eq!(t.correlation_id.as_deref(), Some("c-1"));
        assert_eq!(t.body, OutgoingBody::Bytes(b"<order id=\"1\"/>".to_vec()));
        assert_eq!(
            t.properties["When"],
            TypedValue::DateTime(datetime!(2026-10-17 9:00 UTC))
        );
        assert_eq!(t.time_to_live, Some(Duration::from_secs(600)));
    }

    #[test]
    fn unknown_property_types_are_refused_by_name() {
        let e = parse_template_json(
            r#"{"body":"x","properties":[{"key":"a","type":"Decimal","value":"1"}]}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains("Decimal")
                && e.contains("String, Int32, Int64, Double, Boolean, DateTime, TimeSpan, Guid"),
            "{e}"
        );
    }

    #[test]
    fn empty_strings_are_absent_and_files_are_sniffed() {
        let t = parse_template_json(r#"{"messageId":"","label":"","body":""}"#).unwrap();
        assert_eq!(t.message_id, None);
        assert_eq!(t.subject, None);
        assert_eq!(t.body, OutgoingBody::Empty);
        let dir = tempfile::tempdir().unwrap();
        let j = dir.path().join("t.txt");
        std::fs::write(&j, "  {\"label\":\"L\"}").unwrap();
        assert_eq!(read_template(&j).unwrap().subject.as_deref(), Some("L"));
        let x = dir.path().join("t2.txt");
        std::fs::write(&x, "\n<brokeredMessage><label>X</label></brokeredMessage>").unwrap();
        assert_eq!(read_template(&x).unwrap().subject.as_deref(), Some("X"));
        std::fs::write(&x, "hello").unwrap();
        assert!(read_template(&x).is_err());
    }

    #[test]
    fn xml_keeps_exact_text_of_body_and_values() {
        let t = parse_template_xml(
            "<brokeredMessage>\n  <body>\n  x\n</body>\n  <properties><property><key>k</key><type>String</type><value> v \n</value></property></properties>\n</brokeredMessage>",
        )
        .unwrap();
        assert_eq!(t.body, OutgoingBody::Bytes(b"\n  x\n".to_vec()));
        assert_eq!(t.properties["k"], TypedValue::String(" v \n".into()));
        let t =
            parse_template_xml("<brokeredMessage><body><![CDATA[ a ]]></body></brokeredMessage>")
                .unwrap();
        assert_eq!(t.body, OutgoingBody::Bytes(b" a ".to_vec()));
    }

    #[test]
    fn oversized_template_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.json");
        let body = "x".repeat(MAX_TEMPLATE_BYTES as usize);
        std::fs::write(&p, format!("{{\"body\":\"{body}\"}}")).unwrap();
        let e = read_template(&p).unwrap_err().to_string();
        assert!(e.contains("larger than 1 MiB"), "{e}");
    }

    #[test]
    fn bad_values_are_named() {
        let e = parse_template_json(r#"{"timeToLive":"soon"}"#)
            .unwrap_err()
            .to_string();
        assert!(e.contains("timeToLive"), "{e}");
        let e = parse_template_json(r#"{"scheduledEnqueueTimeUtc":"tomorrow"}"#)
            .unwrap_err()
            .to_string();
        assert!(e.contains("scheduledEnqueueTimeUtc"), "{e}");
        let e = parse_template_json(r#"{"properties":[{"key":"n","type":"Int32","value":"abc"}]}"#)
            .unwrap_err()
            .to_string();
        assert!(e.contains("'n'"), "{e}");
    }
}
