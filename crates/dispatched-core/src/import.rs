//! Reading Dispatched export files (`dispatched.message/v1`) back as messages to send.

use crate::export::EXPORT_SCHEMA;
use crate::messaging::outgoing::copy_properties;
use crate::messaging::props::parse_span;
use crate::messaging::{CloneOptions, OutgoingBody, OutgoingMessage, PropValue, ttl_ms};
use crate::{Error, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The most messages one import file may hold.
pub const MAX_IMPORT_MESSAGES: usize = 100_000;
/// The largest import file, in bytes.
pub const MAX_IMPORT_BYTES: u64 = 256 * 1024 * 1024;

/// A message read from an export file, ready to send.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedMessage {
    /// The sequence number the message had where it was exported.
    pub source_sequence: Option<i64>,
    /// The message to send.
    pub message: OutgoingMessage,
    /// What changed while reading (re-encoded body, dropped properties).
    pub notes: Vec<String>,
}

#[derive(Deserialize)]
struct RawProp {
    #[serde(rename = "type")]
    ty: String,
    #[serde(default)]
    value: serde_json::Value,
}

#[derive(Deserialize, Default)]
struct RawBody {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    value: serde_json::Value,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    schema: Option<String>,
    #[serde(default)]
    sequence_number: Option<i64>,
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    correlation_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    reply_to: Option<String>,
    #[serde(default)]
    reply_to_session_id: Option<String>,
    #[serde(default)]
    partition_key: Option<String>,
    #[serde(default, rename = "time_to_live_ms", with = "ttl_ms")]
    time_to_live: Option<Duration>,
    #[serde(default)]
    application_properties: BTreeMap<String, RawProp>,
    #[serde(default)]
    body: RawBody,
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.filter(|s| !s.is_empty())
}

fn prop_value(name: &str, p: &RawProp) -> Result<PropValue> {
    let bad = |what: &str| {
        Error::Invalid(format!(
            "property '{name}' ({}) is not {what}",
            p.ty.to_ascii_lowercase()
        ))
    };
    let text = || p.value.as_str().ok_or_else(|| bad("text"));
    Ok(match p.ty.to_ascii_lowercase().as_str() {
        "null" => PropValue::Null,
        "bool" => PropValue::Bool(p.value.as_bool().ok_or_else(|| bad("true or false"))?),
        "int" => PropValue::Int(p.value.as_i64().ok_or_else(|| bad("an integer"))?),
        "uint" => PropValue::UInt(p.value.as_u64().ok_or_else(|| bad("an unsigned integer"))?),
        "double" => PropValue::Double(p.value.as_f64().ok_or_else(|| bad("a number"))?),
        "string" => PropValue::String(text()?.to_string()),
        "timestamp" => PropValue::Timestamp(
            OffsetDateTime::parse(text()?, &Rfc3339).map_err(|_| bad("an RFC 3339 time"))?,
        ),
        "timespan" => PropValue::TimeSpan(parse_span(text()?).map_err(|_| bad("a duration"))?),
        "uuid" => PropValue::Uuid(uuid::Uuid::parse_str(text()?).map_err(|_| bad("a GUID"))?),
        "binary" => PropValue::Binary(Vec::new()),
        "other" => PropValue::Other(String::new()),
        other => {
            return Err(Error::Invalid(format!(
                "property '{name}' has unknown type '{other}'"
            )));
        }
    })
}

fn convert(raw: Raw) -> Result<ImportedMessage> {
    let mut notes = Vec::new();
    let body = match raw.body.kind.as_str() {
        "binary" => {
            let s = raw
                .body
                .value
                .as_str()
                .ok_or_else(|| Error::Invalid("binary body must be base64 text".into()))?;
            OutgoingBody::Bytes(
                STANDARD
                    .decode(s)
                    .map_err(|_| Error::Invalid("binary body is not valid base64".into()))?,
            )
        }
        "text" => OutgoingBody::AmqpString(
            raw.body
                .value
                .as_str()
                .ok_or_else(|| Error::Invalid("text body must be a string".into()))?
                .to_string(),
        ),
        "value" | "sequence" => {
            notes.push("body re-encoded from JSON".to_string());
            OutgoingBody::AmqpJson(raw.body.value)
        }
        "empty" | "" => OutgoingBody::Empty,
        other => {
            return Err(Error::Invalid(format!("unknown body kind '{other}'")));
        }
    };
    let mut props = BTreeMap::new();
    for (name, p) in &raw.application_properties {
        props.insert(name.clone(), prop_value(name, p)?);
    }
    let properties = copy_properties(
        &props,
        &CloneOptions {
            keep_dead_letter_properties: true,
            ..Default::default()
        },
        &mut notes,
    );
    let message = OutgoingMessage {
        body,
        message_id: non_empty(raw.message_id),
        session_id: non_empty(raw.session_id),
        correlation_id: non_empty(raw.correlation_id),
        subject: non_empty(raw.subject),
        to: non_empty(raw.to),
        reply_to: non_empty(raw.reply_to),
        reply_to_session_id: non_empty(raw.reply_to_session_id),
        content_type: non_empty(raw.content_type),
        partition_key: non_empty(raw.partition_key),
        time_to_live: raw.time_to_live,
        scheduled_enqueue_time: None,
        properties,
    };
    Ok(ImportedMessage {
        source_sequence: raw.sequence_number,
        message,
        notes,
    })
}

fn read_one(value: serde_json::Value) -> Result<ImportedMessage> {
    let schema = value.get("schema").and_then(|s| s.as_str());
    if schema != Some(EXPORT_SCHEMA) {
        return Err(Error::Invalid(format!(
            "schema is {}; only {EXPORT_SCHEMA} files can be imported",
            schema.map_or("missing".to_string(), |s| format!("'{s}'"))
        )));
    }
    let raw: Raw = serde_json::from_value(value).map_err(|e| Error::Invalid(e.to_string()))?;
    debug_assert_eq!(raw.schema.as_deref(), Some(EXPORT_SCHEMA));
    convert(raw)
}

fn too_many() -> Error {
    Error::Invalid(format!(
        "the file holds more than {MAX_IMPORT_MESSAGES} messages; split it and import the parts"
    ))
}

/// Read a JSON array or NDJSON file of exported messages.
pub fn read_messages(path: &Path) -> Result<Vec<ImportedMessage>> {
    let io = |e: std::io::Error| {
        Error::Io(format!(
            "cannot read {}: {e}; check the path and permissions",
            path.display()
        ))
    };
    let len = std::fs::metadata(path).map_err(io)?.len();
    if len > MAX_IMPORT_BYTES {
        return Err(Error::Invalid(format!(
            "{} is larger than 256 MiB; split it and import the parts",
            path.display()
        )));
    }
    let text = std::fs::read_to_string(path).map_err(io)?;
    let text = text.trim_start_matches('\u{feff}');
    let mut out = Vec::new();
    if text.trim_start().starts_with('[') {
        let items: Vec<serde_json::Value> = serde_json::from_str(text)
            .map_err(|e| Error::Invalid(format!("{} is not a JSON array: {e}", path.display())))?;
        if items.len() > MAX_IMPORT_MESSAGES {
            return Err(too_many());
        }
        for (i, item) in items.into_iter().enumerate() {
            out.push(
                read_one(item).map_err(|e| Error::Invalid(format!("message {}: {e}", i + 1)))?,
            );
        }
    } else {
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            if out.len() >= MAX_IMPORT_MESSAGES {
                return Err(too_many());
            }
            let value: serde_json::Value = serde_json::from_str(line)
                .map_err(|e| Error::Invalid(format!("line {}: not valid JSON: {e}", i + 1)))?;
            out.push(read_one(value).map_err(|e| Error::Invalid(format!("line {}: {e}", i + 1)))?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::ExportFormat;
    use crate::messaging::{MessageBody, TypedValue};

    #[test]
    fn exported_json_and_ndjson_read_back_as_outgoing_messages() {
        let mut m = crate::messaging::test_message(7);
        m.body = MessageBody::Binary(b"hello".to_vec());
        m.message_id = Some("id-7".into());
        m.time_to_live = Some(Duration::from_secs(90));
        m.application_properties
            .insert("n".into(), PropValue::Int(3));
        m.application_properties.insert(
            "wait".into(),
            PropValue::TimeSpan(time::Duration::minutes(90)),
        );
        let dir = tempfile::tempdir().unwrap();
        for format in [ExportFormat::Json, ExportFormat::Ndjson] {
            let path = dir.path().join(format!("{format:?}"));
            crate::export::write_messages(
                std::fs::File::create(&path).unwrap(),
                &[m.clone()],
                format,
            )
            .unwrap();
            let read = read_messages(&path).unwrap();
            assert_eq!(read.len(), 1);
            assert_eq!(read[0].source_sequence, Some(7));
            assert_eq!(read[0].message.body, OutgoingBody::Bytes(b"hello".to_vec()));
            assert_eq!(read[0].message.message_id.as_deref(), Some("id-7"));
            assert_eq!(read[0].message.time_to_live, Some(Duration::from_secs(90)));
            assert_eq!(read[0].message.properties["n"], TypedValue::Int(3));
            assert_eq!(
                read[0].message.properties["wait"],
                TypedValue::TimeSpan(time::Duration::minutes(90)),
                "timespan survives export and import"
            );
        }
    }

    #[test]
    fn a_bad_line_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.ndjson");
        std::fs::write(&p, "{\"schema\":\"dispatched.message/v1\",\"sequence_number\":1,\"state\":\"active\",\"body\":{\"kind\":\"empty\"},\"application_properties\":{},\"delivery_count\":0}\nnot json\n").unwrap();
        let e = read_messages(&p).unwrap_err().to_string();
        assert!(e.contains("line 2"), "{e}");
    }

    #[test]
    fn other_schemas_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.json");
        std::fs::write(&p, r#"[{"schema":"something/v9"}]"#).unwrap();
        let e = read_messages(&p).unwrap_err().to_string();
        assert!(
            e.contains("dispatched.message/v1") && e.contains("message 1"),
            "{e}"
        );
    }

    #[test]
    fn body_kinds_and_unsupported_properties_are_noted() {
        let line = |body: &str, props: &str| {
            format!(
                "{{\"schema\":\"dispatched.message/v1\",\"time_to_live_ms\":null,\"body\":{body},\"application_properties\":{props}}}\n"
            )
        };
        let mut s = String::new();
        s += &line(r#"{"kind":"text","value":"hi"}"#, "{}");
        s += &line(r#"{"kind":"value","value":{"a":1}}"#, "{}");
        s += &line(
            r#"{"kind":"empty"}"#,
            r#"{"b":{"type":"binary","value":"AAE="},"o":{"type":"other","value":"x"},"s":{"type":"string","value":"v"}}"#,
        );
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.ndjson");
        std::fs::write(&p, s).unwrap();
        let r = read_messages(&p).unwrap();
        assert_eq!(r[0].message.body, OutgoingBody::AmqpString("hi".into()));
        assert_eq!(r[0].message.time_to_live, None);
        assert_eq!(
            r[1].message.body,
            OutgoingBody::AmqpJson(serde_json::json!({"a": 1}))
        );
        assert!(
            r[1].notes
                .contains(&"body re-encoded from JSON".to_string())
        );
        assert_eq!(r[2].message.body, OutgoingBody::Empty);
        assert_eq!(r[2].message.properties.len(), 1);
        assert_eq!(r[2].notes.len(), 2, "{:?}", r[2].notes);
    }

    #[test]
    fn missing_ttl_field_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.json");
        std::fs::write(&p, r#"[{"schema":"dispatched.message/v1"}]"#).unwrap();
        let r = read_messages(&p).unwrap();
        assert_eq!(r[0].message.time_to_live, None);
        assert_eq!(r[0].message.body, OutgoingBody::Empty);
    }
}
