//! Stable JSON shape for messages leaving Dispatched (CLI output, files).

use crate::messaging::{MessageBody, PeekedMessage};
use crate::{Error, Result};
use serde::Serialize;
use std::io::Write;

/// Schema identifier written into every exported message.
pub const EXPORT_SCHEMA: &str = "dispatched.message/v1";

/// A message in export form.
#[derive(Debug, Serialize)]
pub struct ExportedMessage<'a> {
    /// Always [`EXPORT_SCHEMA`].
    pub schema: &'static str,
    /// The message fields.
    #[serde(flatten)]
    pub message: &'a PeekedMessage,
    /// The body as text when it is UTF-8 binary (convenience; `body` stays authoritative).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
}

impl<'a> ExportedMessage<'a> {
    /// Wrap `message`.
    pub fn new(message: &'a PeekedMessage) -> Self {
        let body_text = match &message.body {
            MessageBody::Binary(_) => message.body.as_text(),
            _ => None,
        };
        Self {
            schema: EXPORT_SCHEMA,
            message,
            body_text,
        }
    }
}

/// Output layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// One pretty-printed JSON array.
    Json,
    /// One JSON object per line.
    Ndjson,
}

impl ExportFormat {
    /// Parse `json` or `ndjson`.
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(ExportFormat::Json),
            "ndjson" | "jsonl" => Ok(ExportFormat::Ndjson),
            other => Err(Error::Invalid(format!(
                "unknown export format `{other}`; use json or ndjson"
            ))),
        }
    }
}

fn io(e: impl std::fmt::Display) -> Error {
    Error::Io(format!(
        "cannot write export: {e} \u{2014} check the output path, permissions and free space"
    ))
}

/// Write `messages` to `w`. Returns the number written.
pub fn write_messages(
    mut w: impl Write,
    messages: &[PeekedMessage],
    format: ExportFormat,
) -> Result<usize> {
    let exported: Vec<ExportedMessage<'_>> = messages.iter().map(ExportedMessage::new).collect();
    match format {
        ExportFormat::Json => {
            serde_json::to_writer_pretty(&mut w, &exported).map_err(io)?;
            writeln!(w).map_err(io)?;
        }
        ExportFormat::Ndjson => {
            for m in &exported {
                serde_json::to_writer(&mut w, m).map_err(io)?;
                writeln!(w).map_err(io)?;
            }
        }
    }
    w.flush().map_err(io)?;
    Ok(exported.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::{MessageState, test_message};
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;

    fn two() -> Vec<PeekedMessage> {
        let mut a = test_message(1);
        a.body = MessageBody::Binary(b"{\"x\":1}".to_vec());
        let mut b = test_message(2);
        b.body = MessageBody::Binary(vec![0xff, 0x00]);
        vec![a, b]
    }

    fn export_json(messages: &[PeekedMessage]) -> serde_json::Value {
        let mut out = Vec::new();
        write_messages(&mut out, messages, ExportFormat::Json).unwrap();
        serde_json::from_slice(&out).unwrap()
    }

    #[test]
    fn json_array_carries_schema_and_text_convenience() {
        let mut out = Vec::new();
        assert_eq!(
            write_messages(&mut out, &two(), ExportFormat::Json).unwrap(),
            2
        );
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v[0]["schema"], EXPORT_SCHEMA);
        assert_eq!(v[0]["sequence_number"], 1);
        assert_eq!(v[0]["body_text"], "{\"x\":1}");
        assert_eq!(v[0]["body"]["kind"], "binary");
        assert!(v[1].get("body_text").is_none());
    }

    #[test]
    fn json_output_is_a_single_array_of_two() {
        let v = export_json(&two());
        let arr = v.as_array().expect("top-level JSON array");
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn ndjson_is_one_object_per_line() {
        let mut out = Vec::new();
        write_messages(&mut out, &two(), ExportFormat::Ndjson).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(v["schema"], EXPORT_SCHEMA);
        }
    }

    #[test]
    fn ndjson_has_exactly_two_newline_terminated_lines() {
        let mut out = Vec::new();
        write_messages(&mut out, &two(), ExportFormat::Ndjson).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.ends_with('\n'));
        assert_eq!(text.matches('\n').count(), 2, "no other newlines: {text:?}");
        assert!(!text.contains('\r'));
        let seqs: Vec<i64> = text
            .lines()
            .map(|l| {
                serde_json::from_str::<serde_json::Value>(l).unwrap()["sequence_number"]
                    .as_i64()
                    .unwrap()
            })
            .collect();
        assert_eq!(seqs, vec![1, 2]);
    }

    #[test]
    fn exported_state_is_pinned() {
        let mut m = test_message(7);
        m.state = MessageState::Scheduled;
        let v = export_json(&[m]);
        assert_eq!(v[0]["state"], "scheduled");
    }

    #[test]
    fn binary_body_round_trips_through_base64() {
        let bytes = vec![0u8, 1, 2, 254, 255];
        let mut m = test_message(1);
        m.body = MessageBody::Binary(bytes.clone());
        let v = export_json(&[m]);
        assert_eq!(v[0]["body"]["kind"], "binary");
        let encoded = v[0]["body"]["value"].as_str().unwrap();
        assert_eq!(STANDARD.decode(encoded).unwrap(), bytes);
        // Not valid UTF-8, so no text convenience field.
        assert!(v[0].get("body_text").is_none());
    }

    #[test]
    fn enqueued_time_is_rfc3339_and_parses_back() {
        let t = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let mut m = test_message(1);
        m.enqueued_time = Some(t);
        let v = export_json(&[m]);
        let s = v[0]["enqueued_time"].as_str().unwrap();
        assert_eq!(OffsetDateTime::parse(s, &Rfc3339).unwrap(), t);
    }

    #[test]
    fn text_body_has_no_body_text_convenience() {
        let mut m = test_message(1);
        m.body = MessageBody::Text("hello".into());
        let v = export_json(&[m]);
        assert_eq!(
            v[0]["body"],
            serde_json::json!({"kind": "text", "value": "hello"})
        );
        assert!(v[0].get("body_text").is_none());
    }

    #[test]
    fn format_names() {
        assert_eq!(ExportFormat::parse("NDJSON").unwrap(), ExportFormat::Ndjson);
        assert_eq!(ExportFormat::parse("jsonl").unwrap(), ExportFormat::Ndjson);
        assert_eq!(ExportFormat::parse(" json ").unwrap(), ExportFormat::Json);
        assert!(ExportFormat::parse("csv").is_err());
    }

    #[test]
    fn write_failure_is_io_error_with_hint() {
        struct Failing;
        impl Write for Failing {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let err = write_messages(Failing, &two(), ExportFormat::Ndjson).unwrap_err();
        assert!(
            matches!(err, Error::Io(ref m) if m.contains("disk full") && m.contains("free space")),
            "{err}"
        );
    }
}
