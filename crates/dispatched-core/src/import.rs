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
    /// Custom properties the message lacks because their type cannot be sent (`other`).
    pub lost: Vec<String>,
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

/// A JSON integer that fits the signed type `T`.
fn signed<T: TryFrom<i64>>(v: &serde_json::Value) -> Option<T> {
    T::try_from(v.as_i64()?).ok()
}

/// A JSON integer that fits the unsigned type `T`.
fn unsigned<T: TryFrom<u64>>(v: &serde_json::Value) -> Option<T> {
    T::try_from(v.as_u64()?).ok()
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
        "int" | "int32" => {
            let n = p.value.as_i64().ok_or_else(|| bad("an integer"))?;
            // Files written before the `long` tag existed hold wide values under `int`.
            match i32::try_from(n) {
                Ok(n) => PropValue::Int(n),
                Err(_) => PropValue::Long(n),
            }
        }
        "long" | "int64" => PropValue::Long(p.value.as_i64().ok_or_else(|| bad("an integer"))?),
        "byte" => PropValue::Byte(signed(&p.value).ok_or_else(|| bad("an 8-bit integer"))?),
        "short" => PropValue::Short(signed(&p.value).ok_or_else(|| bad("a 16-bit integer"))?),
        "ubyte" => {
            PropValue::UByte(unsigned(&p.value).ok_or_else(|| bad("an 8-bit unsigned integer"))?)
        }
        "ushort" => {
            PropValue::UShort(unsigned(&p.value).ok_or_else(|| bad("a 16-bit unsigned integer"))?)
        }
        "uint" => PropValue::UInt(
            unsigned(&p.value)
                .ok_or_else(|| bad("a 32-bit unsigned integer; use ulong for wider values"))?,
        ),
        "ulong" => PropValue::ULong(p.value.as_u64().ok_or_else(|| bad("an unsigned integer"))?),
        "double" if p.value.is_null() => {
            return Err(Error::Invalid(format!(
                "property '{name}' (double) has no value; NaN and infinity cannot be imported"
            )));
        }
        "double" => PropValue::Double(p.value.as_f64().ok_or_else(|| bad("a number"))?),
        "float" if p.value.is_null() => {
            return Err(Error::Invalid(format!(
                "property '{name}' (float) has no value; NaN and infinity cannot be imported"
            )));
        }
        "float" => {
            let f = p.value.as_f64().ok_or_else(|| bad("a number"))? as f32;
            if !f.is_finite() {
                return Err(bad("a 32-bit float"));
            }
            PropValue::Float(f)
        }
        "symbol" => PropValue::Symbol(text()?.to_string()),
        "string" => PropValue::String(text()?.to_string()),
        "timestamp" => PropValue::Timestamp(
            OffsetDateTime::parse(text()?, &Rfc3339).map_err(|_| bad("an RFC 3339 time"))?,
        ),
        "timespan" => PropValue::TimeSpan(parse_span(text()?).map_err(|_| bad("a duration"))?),
        "uuid" => PropValue::Uuid(uuid::Uuid::parse_str(text()?).map_err(|_| bad("a GUID"))?),
        "binary" => PropValue::Binary(
            base64::engine::general_purpose::STANDARD
                .decode(text()?)
                .map_err(|_| bad("base64"))?,
        ),
        // Rendered text only: it cannot be sent again, and the copy reports it lost.
        "other" => PropValue::Other(p.value.as_str().unwrap_or_default().to_string()),
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
        let value = prop_value(name, p)?;
        props.insert(name.clone(), value);
    }
    let mut lost = Vec::new();
    let properties = copy_properties(
        &props,
        &CloneOptions {
            keep_dead_letter_properties: true,
            ..Default::default()
        },
        &mut notes,
        &mut lost,
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
        lost,
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
    let raw: Raw = match serde_json::from_value(value.clone()) {
        Ok(r) => r,
        // serde's own text can quote the offending value (a body, a property); name the field only.
        Err(_) => return Err(Error::Invalid(shape_error(&value))),
    };
    debug_assert_eq!(raw.schema.as_deref(), Some(EXPORT_SCHEMA));
    convert(raw)
}

fn field_ok<T: serde::de::DeserializeOwned>(v: &serde_json::Value, name: &str) -> bool {
    v.get(name)
        .is_none_or(|f| serde_json::from_value::<T>(f.clone()).is_ok())
}

/// Which field has the wrong shape, without quoting any value.
fn shape_error(v: &serde_json::Value) -> String {
    let bad = if !field_ok::<Option<i64>>(v, "sequence_number") {
        "sequence_number"
    } else if !field_ok::<RawBody>(v, "body") {
        "body"
    } else if !field_ok::<BTreeMap<String, RawProp>>(v, "application_properties") {
        "application_properties"
    } else if v
        .get("time_to_live_ms")
        .is_some_and(|f| !f.is_null() && f.as_u64().is_none())
    {
        "time_to_live_ms"
    } else {
        [
            "message_id",
            "correlation_id",
            "session_id",
            "subject",
            "content_type",
            "to",
            "reply_to",
            "reply_to_session_id",
            "partition_key",
        ]
        .into_iter()
        .find(|n| !field_ok::<Option<String>>(v, n))
        .unwrap_or("message")
    };
    format!("{bad} has the wrong shape")
}

fn too_many(max: usize) -> Error {
    Error::Invalid(format!(
        "the file holds more than {max} messages; split it and import the parts"
    ))
}

/// Read a JSON array or NDJSON file of exported messages.
pub fn read_messages(path: &Path) -> Result<Vec<ImportedMessage>> {
    read_messages_limited(path, MAX_IMPORT_MESSAGES, MAX_IMPORT_BYTES)
}

fn read_messages_limited(
    path: &Path,
    max_messages: usize,
    max_bytes: u64,
) -> Result<Vec<ImportedMessage>> {
    use std::io::Read;
    let io = |e: std::io::Error| {
        Error::Io(format!(
            "cannot read {}: {e}; check the path and permissions",
            path.display()
        ))
    };
    // Read through take() so a FIFO or other special file cannot slip past the size cap.
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(io)?
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > max_bytes {
        return Err(Error::Invalid(format!(
            "{} is larger than {} MiB; split it and import the parts",
            path.display(),
            max_bytes / (1024 * 1024)
        )));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| Error::Invalid(format!("{} is not UTF-8 text", path.display())))?;
    let text = text.trim_start_matches('\u{feff}');
    let mut out = Vec::new();
    if text.trim_start().starts_with('[') {
        let items: Vec<serde_json::Value> = serde_json::from_str(text)
            .map_err(|e| Error::Invalid(format!("{} is not a JSON array: {e}", path.display())))?;
        if items.len() > max_messages {
            return Err(too_many(max_messages));
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
            if out.len() >= max_messages {
                return Err(too_many(max_messages));
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
        let blob: Vec<u8> = (0..=255).cycle().take(4096).collect();
        m.application_properties
            .insert("blob".into(), PropValue::Binary(blob.clone()));
        m.application_properties
            .insert("none".into(), PropValue::Null);
        m.application_properties
            .insert("b".into(), PropValue::UByte(255));
        m.application_properties
            .insert("s".into(), PropValue::UShort(65_535));
        m.application_properties
            .insert("u".into(), PropValue::UInt(u32::MAX));
        m.application_properties
            .insert("ul".into(), PropValue::ULong(u64::MAX));
        m.application_properties
            .insert("sb".into(), PropValue::Byte(i8::MIN));
        m.application_properties
            .insert("ss".into(), PropValue::Short(i16::MIN));
        m.application_properties
            .insert("f".into(), PropValue::Float(1.5));
        m.application_properties
            .insert("sym".into(), PropValue::Symbol("y".into()));
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
            let p = &read[0].message.properties;
            assert_eq!(
                p["blob"],
                TypedValue::Binary(blob.clone()),
                "binary in full"
            );
            assert_eq!(p["none"], TypedValue::Null);
            assert_eq!(p["b"], TypedValue::UByte(255));
            assert_eq!(p["s"], TypedValue::UShort(65_535));
            assert_eq!(p["u"], TypedValue::UInt(u32::MAX));
            assert_eq!(p["ul"], TypedValue::ULong(u64::MAX));
            assert_eq!(p["sb"], TypedValue::Byte(i8::MIN));
            assert_eq!(p["ss"], TypedValue::Short(i16::MIN));
            assert_eq!(p["f"], TypedValue::Float(1.5));
            assert_eq!(p["sym"], TypedValue::Symbol("y".into()));
            assert!(read[0].notes.is_empty(), "{:?}", read[0].notes);
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
        assert_eq!(r[2].message.properties.len(), 2, "binary and string");
        assert_eq!(r[2].message.properties["b"], TypedValue::Binary(vec![0, 1]));
        assert_eq!(
            r[2].notes,
            ["property o (unsupported type) was not copied"],
            "only `other` is lost"
        );
        assert_eq!(r[2].lost, ["o"]);
    }

    #[test]
    fn a_uint_too_wide_suggests_ulong() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.ndjson");
        std::fs::write(
            &p,
            "{\"schema\":\"dispatched.message/v1\",\"body\":{\"kind\":\"empty\"},\"application_properties\":{\"n\":{\"type\":\"uint\",\"value\":5000000000}}}\n",
        )
        .unwrap();
        let e = read_messages(&p).unwrap_err().to_string();
        assert!(e.contains("use ulong"), "{e}");
    }

    #[test]
    fn shape_errors_never_echo_values() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.ndjson");
        for bad in [
            r#"{"schema":"dispatched.message/v1","body":"SECRET-body"}"#,
            r#"{"schema":"dispatched.message/v1","application_properties":"SECRET-body"}"#,
            r#"{"schema":"dispatched.message/v1","time_to_live_ms":"SECRET-body"}"#,
            r#"{"schema":"dispatched.message/v1","subject":{"SECRET-body":1}}"#,
            r#"{"schema":"dispatched.message/v1","body":{"kind":"text","value":"ok"},"sequence_number":"SECRET-body"}"#,
        ] {
            std::fs::write(&p, format!("{bad}\n")).unwrap();
            let e = read_messages(&p).unwrap_err().to_string();
            assert!(!e.contains("SECRET"), "{e}");
            assert!(e.contains("line 1") && e.contains("wrong shape"), "{e}");
        }
        std::fs::write(
            &p,
            "{\"schema\":\"dispatched.message/v1\",\"body\":\"SECRET-body\"}\n",
        )
        .unwrap();
        assert!(
            read_messages(&p)
                .unwrap_err()
                .to_string()
                .contains("body has the wrong shape")
        );
    }

    #[test]
    fn long_tag_keeps_64_bits_and_nan_is_refused_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.ndjson");
        std::fs::write(
            &p,
            r#"{"schema":"dispatched.message/v1","application_properties":{"a":{"type":"long","value":5},"b":{"type":"int","value":5}}}"#,
        )
        .unwrap();
        let r = read_messages(&p).unwrap();
        assert_eq!(r[0].message.properties["a"], TypedValue::Long(5));
        assert_eq!(r[0].message.properties["b"], TypedValue::Int(5));
        std::fs::write(
            &p,
            r#"{"schema":"dispatched.message/v1","application_properties":{"d":{"type":"double","value":null}}}"#,
        )
        .unwrap();
        let e = read_messages(&p).unwrap_err().to_string();
        assert!(e.contains("'d'") && e.contains("NaN"), "{e}");
    }

    #[test]
    fn caps_on_messages_and_bytes_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let line = "{\"schema\":\"dispatched.message/v1\"}\n";
        let nd = dir.path().join("a.ndjson");
        std::fs::write(&nd, line.repeat(3)).unwrap();
        assert_eq!(read_messages_limited(&nd, 3, 1 << 20).unwrap().len(), 3);
        let e = read_messages_limited(&nd, 2, 1 << 20)
            .unwrap_err()
            .to_string();
        assert!(e.contains("more than 2 messages"), "{e}");
        let js = dir.path().join("a.json");
        std::fs::write(
            &js,
            format!(
                "[{}]",
                ["{\"schema\":\"dispatched.message/v1\"}"; 3].join(",")
            ),
        )
        .unwrap();
        assert!(read_messages_limited(&js, 2, 1 << 20).is_err());
        let e = read_messages_limited(&nd, 10, 10).unwrap_err().to_string();
        assert!(e.contains("larger than"), "{e}");
        assert_eq!(MAX_IMPORT_MESSAGES, 100_000);
        assert_eq!(MAX_IMPORT_BYTES, 256 * 1024 * 1024);
    }

    #[test]
    fn a_small_long_stays_a_long_from_decode_through_export_import_and_encode() {
        use fe2o3_amqp::types::primitives::SimpleValue;
        let mut m = crate::messaging::test_message(1);
        m.application_properties.insert(
            "l".into(),
            crate::messaging::simple_to_prop(&SimpleValue::Long(5)),
        );
        m.application_properties.insert(
            "i".into(),
            crate::messaging::simple_to_prop(&SimpleValue::Int(5)),
        );
        assert_eq!(m.application_properties["l"], PropValue::Long(5));
        assert_eq!(m.application_properties["i"], PropValue::Int(5));
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.ndjson");
        crate::export::write_messages(
            std::fs::File::create(&p).unwrap(),
            &[m],
            ExportFormat::Ndjson,
        )
        .unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            text.contains(r#""l":{"type":"long","value":5}"#)
                && text.contains(r#""i":{"type":"int","value":5}"#),
            "{text}"
        );
        let r = read_messages(&p).unwrap();
        assert_eq!(r[0].message.properties["l"], TypedValue::Long(5));
        assert_eq!(r[0].message.properties["i"], TypedValue::Int(5));
        let amqp = r[0].message.to_amqp(OffsetDateTime::now_utc()).unwrap();
        let ap = amqp.application_properties.unwrap().0;
        assert!(matches!(ap.get("l"), Some(SimpleValue::Long(5))));
        assert!(matches!(ap.get("i"), Some(SimpleValue::Int(5))));
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
