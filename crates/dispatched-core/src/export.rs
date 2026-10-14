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
    Error::Invalid(format!("cannot write export: {e}"))
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
    use crate::messaging::test_message;

    fn two() -> Vec<PeekedMessage> {
        let mut a = test_message(1);
        a.body = MessageBody::Binary(b"{\"x\":1}".to_vec());
        let mut b = test_message(2);
        b.body = MessageBody::Binary(vec![0xff, 0x00]);
        vec![a, b]
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
    fn format_names() {
        assert_eq!(ExportFormat::parse("NDJSON").unwrap(), ExportFormat::Ndjson);
        assert!(ExportFormat::parse("csv").is_err());
    }
}
