//! What surfaces show in lists (summaries, no body) and in a detail view (the body, capped).

use super::message::{MessageBody, MessageState, PeekedMessage, PropValue};
use serde::Serialize;
use std::collections::BTreeMap;
use time::OffsetDateTime;

/// Characters of body text kept in a summary's preview.
pub const PREVIEW_CHARS: usize = 160;
/// Largest body a detail view carries; larger bodies are cut and flagged.
pub const DETAIL_BODY_CAP: usize = 4 * 1024 * 1024;

/// A peeked message without its body, for grids and lists.
#[derive(Debug, Clone, Serialize)]
pub struct MessageSummary {
    /// Broker sequence number.
    pub sequence_number: i64,
    /// Which view the message was peeked from.
    pub state: MessageState,
    /// Application message id.
    pub message_id: Option<String>,
    /// Correlation id.
    pub correlation_id: Option<String>,
    /// Session id.
    pub session_id: Option<String>,
    /// Subject (label).
    pub subject: Option<String>,
    /// Content type.
    pub content_type: Option<String>,
    /// `to` address.
    pub to: Option<String>,
    /// `reply-to` address.
    pub reply_to: Option<String>,
    /// When the broker enqueued it.
    #[serde(with = "time::serde::rfc3339::option")]
    pub enqueued_time: Option<OffsetDateTime>,
    /// When it becomes visible, if scheduled.
    #[serde(with = "time::serde::rfc3339::option")]
    pub scheduled_enqueue_time: Option<OffsetDateTime>,
    /// Delivery count.
    pub delivery_count: u32,
    /// Time to live in milliseconds.
    pub time_to_live_ms: Option<u32>,
    /// Dead-letter reason.
    pub dead_letter_reason: Option<String>,
    /// Dead-letter description.
    pub dead_letter_description: Option<String>,
    /// Entity the message was dead-lettered from.
    pub dead_letter_source: Option<String>,
    /// Application properties.
    pub application_properties: BTreeMap<String, PropValue>,
    /// `binary`, `text`, `value`, `sequence` or `empty`.
    pub body_kind: &'static str,
    /// Body size in bytes (see [`MessageBody::len`]).
    pub body_size: usize,
    /// The first [`PREVIEW_CHARS`] characters of the body text with whitespace runs collapsed;
    /// `None` for empty and non-UTF-8 bodies.
    pub body_preview: Option<String>,
}

fn kind(b: &MessageBody) -> &'static str {
    match b {
        MessageBody::Binary(_) => "binary",
        MessageBody::Text(_) => "text",
        MessageBody::Value(_) => "value",
        MessageBody::Sequence(_) => "sequence",
        MessageBody::Empty => "empty",
    }
}

fn preview(b: &MessageBody) -> Option<String> {
    let text = match b {
        // Only look at the head of a large binary body.
        MessageBody::Binary(bytes) => {
            let head = &bytes[..bytes.len().min(PREVIEW_CHARS * 4)];
            match std::str::from_utf8(head) {
                Ok(s) => s.to_string(),
                // A multi-byte character cut at the end of the head is still text.
                Err(e) if e.error_len().is_none() => {
                    String::from_utf8_lossy(&head[..e.valid_up_to()]).into_owned()
                }
                Err(_) => return None,
            }
        }
        other => other.as_text()?,
    };
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then(|| collapsed.chars().take(PREVIEW_CHARS).collect())
}

impl From<&PeekedMessage> for MessageSummary {
    fn from(m: &PeekedMessage) -> Self {
        Self {
            sequence_number: m.sequence_number,
            state: m.state,
            message_id: m.message_id.clone(),
            correlation_id: m.correlation_id.clone(),
            session_id: m.session_id.clone(),
            subject: m.subject.clone(),
            content_type: m.content_type.clone(),
            to: m.to.clone(),
            reply_to: m.reply_to.clone(),
            enqueued_time: m.enqueued_time,
            scheduled_enqueue_time: m.scheduled_enqueue_time,
            delivery_count: m.delivery_count,
            time_to_live_ms: m.time_to_live_ms,
            dead_letter_reason: m.dead_letter_reason.clone(),
            dead_letter_description: m.dead_letter_description.clone(),
            dead_letter_source: m.dead_letter_source.clone(),
            application_properties: m.application_properties.clone(),
            body_kind: kind(&m.body),
            body_size: m.body.len(),
            body_preview: preview(&m.body),
        }
    }
}

/// One message for a detail view, its body cut to a cap.
#[derive(Debug, Clone, Serialize)]
pub struct MessageDetail {
    #[serde(flatten)]
    pub message: PeekedMessage,
    /// The full body size in bytes.
    pub body_size: usize,
    /// The body was cut to the cap.
    pub body_truncated: bool,
}

impl MessageDetail {
    /// Wrap `message`, cutting a body longer than `cap` bytes (text on a character boundary;
    /// value and sequence bodies become their compact JSON text, cut).
    pub fn new(mut message: PeekedMessage, cap: usize) -> Self {
        let body_size = message.body.len();
        let body_truncated = body_size > cap;
        if body_truncated {
            let cut = |s: &str| -> String {
                let mut end = cap.min(s.len());
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                s[..end].to_string()
            };
            message.body = match std::mem::replace(&mut message.body, MessageBody::Empty) {
                MessageBody::Binary(mut b) => {
                    b.truncate(cap);
                    MessageBody::Binary(b)
                }
                MessageBody::Text(s) => MessageBody::Text(cut(&s)),
                MessageBody::Value(j) | MessageBody::Sequence(j) => {
                    MessageBody::Text(cut(&j.to_string()))
                }
                MessageBody::Empty => MessageBody::Empty,
            };
        }
        Self {
            message,
            body_size,
            body_truncated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::{MessageBody, test_message};

    #[test]
    fn summary_previews_text_and_flags_binary() {
        let mut m = test_message(7);
        m.body = MessageBody::Binary(
            format!("{{\"id\":1,\n  \"x\":\"{}\"}}", "y".repeat(500)).into_bytes(),
        );
        let s = MessageSummary::from(&m);
        assert_eq!(s.sequence_number, 7);
        assert_eq!(s.body_kind, "binary");
        assert_eq!(s.body_size, m.body.len());
        let p = s.body_preview.unwrap();
        assert!(
            p.starts_with("{\"id\":1, \"x\""),
            "whitespace collapsed: {p}"
        );
        assert_eq!(p.chars().count(), PREVIEW_CHARS);
        m.body = MessageBody::Binary(vec![0xff, 0xfe, 0x00]);
        let s = MessageSummary::from(&m);
        assert_eq!(s.body_preview, None, "non-UTF-8 has no text preview");
        m.body = MessageBody::Empty;
        assert_eq!(MessageSummary::from(&m).body_kind, "empty");
    }

    #[test]
    fn summary_never_carries_the_body() {
        let mut m = test_message(1);
        m.body = MessageBody::Text("z".repeat(100_000));
        let json = serde_json::to_string(&MessageSummary::from(&m)).unwrap();
        assert!(json.len() < 2_000, "{} bytes", json.len());
        assert!(!json.contains("\"body\""));
    }

    #[test]
    fn detail_truncates_large_bodies_on_char_boundaries() {
        let mut m = test_message(1);
        m.body = MessageBody::Text("é".repeat(10)); // 20 bytes
        let d = MessageDetail::new(m.clone(), 5);
        assert!(d.body_truncated);
        assert_eq!(d.body_size, 20);
        assert_eq!(
            d.message.body,
            MessageBody::Text("éé".into()),
            "cut before the split character"
        );
        m.body = MessageBody::Binary(vec![1; 10]);
        let d = MessageDetail::new(m.clone(), 4);
        assert_eq!(d.message.body, MessageBody::Binary(vec![1; 4]));
        m.body = MessageBody::Value(serde_json::json!({"k": "v".repeat(20)}));
        let d = MessageDetail::new(m.clone(), 8);
        assert!(matches!(d.message.body, MessageBody::Text(ref t) if t.len() <= 8));
        let small = MessageDetail::new(m.clone(), DETAIL_BODY_CAP);
        assert!(!small.body_truncated);
        assert_eq!(small.message.body, m.body);
    }

    #[test]
    fn detail_serializes_flat() {
        let d = MessageDetail::new(test_message(3), DETAIL_BODY_CAP);
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["sequence_number"], 3);
        assert_eq!(v["body_truncated"], false);
        assert_eq!(v["body"]["kind"], "text");
    }
}
