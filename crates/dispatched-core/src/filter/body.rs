//! Filters on message body content.

use crate::messaging::MessageBody;
use crate::{Error, Result};
use serde_json_path::JsonPath;

/// Bodies larger than this are not searched (keeps regex and JSON parsing bounded).
const MAX_SEARCH_BYTES: usize = 4 * 1024 * 1024;

/// A predicate on the message body.
#[derive(Debug, Clone)]
pub enum BodyFilter {
    /// Case-insensitive substring.
    Contains(String),
    /// Regular expression (Rust `regex` syntax) on the body text.
    Regex(regex::Regex),
    /// JSONPath (RFC 9535) on a JSON body; optionally a node must equal `expected`.
    Json {
        /// Parsed path.
        path: JsonPath,
        /// Original path text.
        source: String,
        /// Required node value.
        expected: Option<serde_json::Value>,
    },
}

impl BodyFilter {
    /// Substring match (case-insensitive).
    pub fn contains(text: &str) -> Self {
        BodyFilter::Contains(text.to_lowercase())
    }

    /// Regex match.
    pub fn regex(pattern: &str) -> Result<Self> {
        regex::RegexBuilder::new(pattern)
            .size_limit(1 << 20)
            .build()
            .map(BodyFilter::Regex)
            .map_err(|e| Error::Invalid(format!("body regex: {e}")))
    }

    /// JSONPath match, e.g. `$.order.status` with expected `"shipped"`.
    pub fn json(path: &str, expected: Option<&str>) -> Result<Self> {
        let parsed = JsonPath::parse(path)
            .map_err(|e| Error::Invalid(format!("body JSONPath `{path}`: {e}")))?;
        let expected = expected.map(|v| {
            serde_json::from_str(v).unwrap_or_else(|_| serde_json::Value::String(v.to_string()))
        });
        Ok(BodyFilter::Json {
            path: parsed,
            source: path.to_string(),
            expected,
        })
    }

    /// Whether `body` satisfies this filter. Non-text bodies never match.
    pub fn matches(&self, body: &MessageBody) -> bool {
        if body.len() > MAX_SEARCH_BYTES {
            return false;
        }
        let Some(text) = body.as_text() else {
            return false;
        };
        match self {
            BodyFilter::Contains(needle) => text.to_lowercase().contains(needle.as_str()),
            BodyFilter::Regex(re) => re.is_match(&text),
            BodyFilter::Json { path, expected, .. } => {
                let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else {
                    return false;
                };
                let nodes = path.query(&doc).all();
                match expected {
                    None => !nodes.is_empty(),
                    Some(want) => nodes.iter().any(|n| json_equal(n, want)),
                }
            }
        }
    }
}

fn json_equal(node: &serde_json::Value, want: &serde_json::Value) -> bool {
    match (node, want) {
        (serde_json::Value::Number(a), serde_json::Value::Number(b)) => a.as_f64() == b.as_f64(),
        (serde_json::Value::String(a), serde_json::Value::Number(b)) => {
            a.parse::<f64>().ok() == b.as_f64()
        }
        _ => *node == *want,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> MessageBody {
        MessageBody::Text(s.into())
    }

    #[test]
    fn contains_is_case_insensitive() {
        assert!(BodyFilter::contains("ORDER").matches(&text("an order arrived")));
        assert!(!BodyFilter::contains("refund").matches(&text("an order arrived")));
    }

    #[test]
    fn regex_matches_text_and_rejects_bad_patterns() {
        assert!(
            BodyFilter::regex(r"id=\d{3}")
                .unwrap()
                .matches(&text("x id=123 y"))
        );
        assert!(
            BodyFilter::regex("(")
                .unwrap_err()
                .to_string()
                .contains("body regex")
        );
    }

    #[test]
    fn json_path_presence_and_equality() {
        let body = text(r#"{"order":{"status":"shipped","qty":"2","lines":[{"sku":"A"}]}}"#);
        assert!(
            BodyFilter::json("$.order.status", None)
                .unwrap()
                .matches(&body)
        );
        assert!(
            BodyFilter::json("$.order.status", Some("shipped"))
                .unwrap()
                .matches(&body)
        );
        assert!(
            BodyFilter::json("$.order.qty", Some("2"))
                .unwrap()
                .matches(&body)
        );
        assert!(
            BodyFilter::json("$.order.lines[*].sku", Some("\"A\""))
                .unwrap()
                .matches(&body)
        );
        assert!(
            !BodyFilter::json("$.order.status", Some("open"))
                .unwrap()
                .matches(&body)
        );
        assert!(!BodyFilter::json("$.missing", None).unwrap().matches(&body));
        assert!(
            !BodyFilter::json("$.order", None)
                .unwrap()
                .matches(&text("not json"))
        );
        assert!(BodyFilter::json("$[", None).is_err());
    }

    #[test]
    fn body_filters_ignore_binary_bodies() {
        let binary = MessageBody::Binary(vec![0xff, 0xfe, 0x00]);
        assert!(!BodyFilter::contains("x").matches(&binary));
        assert!(!BodyFilter::regex(".*").unwrap().matches(&binary));
    }

    #[test]
    fn oversized_bodies_are_not_searched() {
        let big = "a".repeat(MAX_SEARCH_BYTES + 1);
        assert!(!BodyFilter::contains("a").matches(&text(&big)));
        assert!(
            !BodyFilter::regex("a")
                .unwrap()
                .matches(&MessageBody::Binary(big.into_bytes()))
        );
        let at_cap = "a".repeat(MAX_SEARCH_BYTES);
        assert!(BodyFilter::contains("a").matches(&text(&at_cap)));
    }

    #[test]
    fn oversized_regex_is_rejected_not_compiled() {
        let e = BodyFilter::regex(r"(\w{100}){100}")
            .unwrap_err()
            .to_string();
        assert!(e.contains("body regex"), "{e}");
    }

    #[test]
    fn hostile_bodies_never_panic() {
        let deep = format!("{}1{}", "[".repeat(10_000), "]".repeat(10_000));
        assert!(
            !BodyFilter::json("$..*", None)
                .unwrap()
                .matches(&text(&deep))
        );
        assert!(!BodyFilter::contains("x").matches(&MessageBody::Empty));
        assert!(
            !BodyFilter::json("$", None)
                .unwrap()
                .matches(&MessageBody::Empty)
        );
        // Valid UTF-8 in a binary section is still text.
        assert!(BodyFilter::contains("hi").matches(&MessageBody::Binary(b"say hi".to_vec())));
        // Non-ASCII case folding does not panic.
        assert!(BodyFilter::contains("STRASSE").matches(&text("Stra\u{df}e strasse")));
        // Structured bodies are searched as JSON.
        let v = MessageBody::Value(serde_json::json!({"a": {"b": 1}}));
        assert!(BodyFilter::json("$.a.b", Some("1")).unwrap().matches(&v));
    }
}
