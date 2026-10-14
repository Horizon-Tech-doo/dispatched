//! Filters on message body content.

use crate::messaging::MessageBody;
use crate::{Error, Result};
use serde_json_path::JsonPath;

/// Bodies larger than this are not searched (keeps regex and JSON parsing bounded).
const MAX_SEARCH_BYTES: usize = 4 * 1024 * 1024;

/// Longest accepted JSONPath text. The parser recurses on nested filters, so input is bounded.
const MAX_PATH_BYTES: usize = 1024;
/// Deepest accepted `(`/`[` nesting in a JSONPath.
const MAX_PATH_DEPTH: usize = 32;
/// Most descendant (`..`) segments accepted; chained ones are polynomial on large bodies.
const MAX_DESCENDANTS: usize = 4;

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
            .dfa_size_limit(2 << 20)
            .build()
            .map(BodyFilter::Regex)
            .map_err(|e| Error::Invalid(format!("body regex: {e}")))
    }

    /// JSONPath match, e.g. `$.order.status` with expected `"shipped"`.
    pub fn json(path: &str, expected: Option<&str>) -> Result<Self> {
        check_path_bounds(path)?;
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
                // The node list is built eagerly by the library; we only avoid copying it.
                let nodes = path.query(&doc);
                match expected {
                    None => !nodes.is_empty(),
                    Some(want) => nodes.iter().any(|n| json_equal(n, want)),
                }
            }
        }
    }
}

/// Reject paths that could overflow the parser's stack or blow up evaluation time.
fn check_path_bounds(path: &str) -> Result<()> {
    if path.len() > MAX_PATH_BYTES {
        return Err(Error::Invalid(format!(
            "body JSONPath is too long (max {MAX_PATH_BYTES} bytes); use a shorter path"
        )));
    }
    let (mut depth, mut descendants) = (0usize, 0usize);
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut prev_dot = false;
    for c in path.chars() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '(' | '[' => {
                depth += 1;
                if depth > MAX_PATH_DEPTH {
                    return Err(Error::Invalid(format!(
                        "body JSONPath is nested too deeply (max {MAX_PATH_DEPTH} levels); simplify the path"
                    )));
                }
            }
            ')' | ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if c == '.' && prev_dot {
            descendants += 1;
            if descendants > MAX_DESCENDANTS {
                return Err(Error::Invalid(format!(
                    "body JSONPath has too many `..` segments (max {MAX_DESCENDANTS}); simplify the path"
                )));
            }
            prev_dot = false; // `...` counts as one segment plus a dot
        } else {
            prev_dot = c == '.';
        }
    }
    Ok(())
}

fn json_equal(node: &serde_json::Value, want: &serde_json::Value) -> bool {
    match (node, want) {
        (serde_json::Value::Number(a), serde_json::Value::Number(b)) => {
            if let (Some(x), Some(y)) = (a.as_i64(), b.as_i64()) {
                x == y
            } else if let (Some(x), Some(y)) = (a.as_u64(), b.as_u64()) {
                x == y
            } else if a.is_f64() || b.is_f64() {
                a.as_f64() == b.as_f64()
            } else {
                // One is a negative i64 and the other a u64 above i64::MAX: never equal.
                false
            }
        }
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
        // Matching is lowercase equality, not full Unicode folding: "STRASSE" does not match "Straße".
        assert!(!BodyFilter::contains("STRASSE").matches(&text("Stra\u{df}e")));
        assert!(BodyFilter::contains("STRA\u{df}E").matches(&text("stra\u{df}e")));
        // Structured bodies are searched as JSON.
        let v = MessageBody::Value(serde_json::json!({"a": {"b": 1}}));
        assert!(BodyFilter::json("$.a.b", Some("1")).unwrap().matches(&v));
    }

    #[test]
    fn oversized_or_deep_paths_are_rejected_before_parsing() {
        let parens = format!("$[?{}", "(".repeat(100_000));
        let e = BodyFilter::json(&parens, None).unwrap_err().to_string();
        assert!(e.contains("too long"), "{e}");
        let deep = format!("$[?{}1{}]", "(".repeat(40), ")".repeat(40));
        let e = BodyFilter::json(&deep, None).unwrap_err().to_string();
        assert!(e.contains("nested too deeply"), "{e}");
        let long = format!("$.{}", "a".repeat(2000));
        assert!(
            BodyFilter::json(&long, None)
                .unwrap_err()
                .to_string()
                .contains("too long")
        );
    }

    #[test]
    fn descendant_segments_are_bounded_and_quotes_are_ignored() {
        let e = BodyFilter::json("$..a..b..c..d..e", None)
            .unwrap_err()
            .to_string();
        assert!(e.contains("`..`"), "{e}");
        assert!(BodyFilter::json("$..a..b", None).is_ok());
        assert!(BodyFilter::json("$..a..b..c..d", None).is_ok());
        // `..` and brackets inside quoted names are not counted.
        let quoted = "$['a..b..c..d..e..f..g']['(((((((((((((((((((((((((((((((((((((((((']";
        assert!(BodyFilter::json(quoted, None).is_ok());
        let body = text(r#"{"a..b..c..d..e..f..g":1}"#);
        assert!(
            BodyFilter::json("$['a..b..c..d..e..f..g']", Some("1"))
                .unwrap()
                .matches(&body)
        );
    }

    #[test]
    fn large_integers_compare_exactly() {
        let body = text(r#"{"id":9007199254740993}"#);
        assert!(
            BodyFilter::json("$.id", Some("9007199254740993"))
                .unwrap()
                .matches(&body)
        );
        assert!(
            !BodyFilter::json("$.id", Some("9007199254740992"))
                .unwrap()
                .matches(&body)
        );
        assert!(
            BodyFilter::json("$.n", Some("2.0"))
                .unwrap()
                .matches(&text(r#"{"n":2}"#))
        );
    }
}
