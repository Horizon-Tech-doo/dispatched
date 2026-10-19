//! A filter as a surface submits it (plain strings), checked and compiled here so every
//! surface reports the same problems.

use super::{BodyFilter, MessageFilter, PropertyFilter};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// Filter fields as typed by a user. Blank strings mean "no condition".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterSpec {
    /// Property expression, e.g. `sys.DeliveryCount > 3 AND region = 'eu'`.
    pub property: Option<String>,
    /// Case-insensitive body substring (spaces are kept).
    pub body_contains: Option<String>,
    /// Body regular expression.
    pub body_regex: Option<String>,
    /// JSONPath into a JSON body.
    pub body_json_path: Option<String>,
    /// With `body_json_path`: the value a node must equal (JSON, or a bare string).
    pub body_json_equals: Option<String>,
    /// Only this session's messages (exact).
    pub session_id: Option<String>,
}

/// The field a problem belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterField {
    /// `property`
    Property,
    /// `body_regex`
    BodyRegex,
    /// `body_json_path`
    BodyJsonPath,
    /// `body_json_equals`
    BodyJsonEquals,
}

/// One problem with a [`FilterSpec`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FilterIssue {
    /// Which field.
    pub field: FilterField,
    /// What is wrong, in the words the parsers use (property errors name the column).
    pub message: String,
}

fn trimmed(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|t| !t.is_empty())
}

/// The original string unless it is blank (trimming only decides blankness; the text is kept).
fn unless_blank(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|t| !t.trim().is_empty())
}

fn raw(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|t| !t.is_empty())
}

impl FilterSpec {
    /// No field is set.
    pub fn is_empty(&self) -> bool {
        trimmed(&self.property).is_none()
            && raw(&self.body_contains).is_none()
            && unless_blank(&self.body_regex).is_none()
            && trimmed(&self.body_json_path).is_none()
            && unless_blank(&self.body_json_equals).is_none()
            && raw(&self.session_id).is_none()
    }

    /// Compile every field, or return every problem (at most one per field).
    pub fn compile(&self) -> std::result::Result<MessageFilter, Vec<FilterIssue>> {
        let mut issues = Vec::new();
        let mut issue = |field, e: Error| {
            issues.push(FilterIssue {
                field,
                message: e.to_string(),
            })
        };
        let mut f = MessageFilter::default();
        if let Some(p) = trimmed(&self.property) {
            match PropertyFilter::parse(p) {
                Ok(pf) => f.property = Some(pf),
                Err(e) => issue(FilterField::Property, e),
            }
        }
        if let Some(t) = raw(&self.body_contains) {
            f.body.push(BodyFilter::contains(t));
        }
        if let Some(r) = unless_blank(&self.body_regex) {
            match BodyFilter::regex(r) {
                Ok(b) => f.body.push(b),
                Err(e) => issue(FilterField::BodyRegex, e),
            }
        }
        match (
            trimmed(&self.body_json_path),
            unless_blank(&self.body_json_equals),
        ) {
            (Some(path), eq) => match BodyFilter::json(path, eq) {
                Ok(b) => f.body.push(b),
                Err(e) => issue(FilterField::BodyJsonPath, e),
            },
            (None, Some(_)) => issue(
                FilterField::BodyJsonEquals,
                Error::Invalid("add a JSONPath to compare this value with".into()),
            ),
            (None, None) => {}
        }
        f.session_id = raw(&self.session_id).map(str::to_string);
        if issues.is_empty() {
            Ok(f)
        } else {
            Err(issues)
        }
    }

    /// Compile, failing with the first problem.
    pub fn build(&self) -> Result<MessageFilter> {
        self.compile().map_err(|issues| {
            Error::Invalid(
                issues
                    .into_iter()
                    .next()
                    .map(|i| i.message)
                    .unwrap_or_default(),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_fields_mean_no_condition() {
        let s = FilterSpec {
            property: Some("  ".into()),
            body_regex: Some(String::new()),
            ..Default::default()
        };
        assert!(s.is_empty());
        assert!(s.build().unwrap().is_empty());
    }

    #[test]
    fn every_field_compiles() {
        let s = FilterSpec {
            property: Some("sys.DeliveryCount > 1".into()),
            body_contains: Some("order".into()),
            body_regex: Some("^\\{".into()),
            body_json_path: Some("$.status".into()),
            body_json_equals: Some("\"shipped\"".into()),
            session_id: Some("RMA-1".into()),
        };
        let f = s.build().unwrap();
        assert!(f.property.is_some());
        assert_eq!(f.body.len(), 3);
        assert_eq!(f.session_id.as_deref(), Some("RMA-1"));
    }

    #[test]
    fn each_bad_field_is_reported_once_with_its_field() {
        let s = FilterSpec {
            property: Some("a =".into()),
            body_regex: Some("(".into()),
            body_json_equals: Some("1".into()),
            ..Default::default()
        };
        let issues = s.compile().unwrap_err();
        let fields: Vec<FilterField> = issues.iter().map(|i| i.field).collect();
        assert_eq!(
            fields,
            [
                FilterField::Property,
                FilterField::BodyRegex,
                FilterField::BodyJsonEquals
            ]
        );
        assert!(
            issues[0].message.contains("column"),
            "{}",
            issues[0].message
        );
        assert!(s.build().unwrap_err().to_string().contains("column"));
    }

    #[test]
    fn bad_json_path_is_its_own_field() {
        let s = FilterSpec {
            body_json_path: Some("$[".into()),
            ..Default::default()
        };
        assert_eq!(s.compile().unwrap_err()[0].field, FilterField::BodyJsonPath);
    }

    #[test]
    fn contains_keeps_its_spaces_and_session_is_not_trimmed() {
        let s = FilterSpec {
            body_contains: Some(" id".into()),
            session_id: Some(" a".into()),
            ..Default::default()
        };
        let f = s.build().unwrap();
        let mut m = crate::messaging::test_message(1);
        m.body = crate::MessageBody::Text("order id".into());
        m.session_id = Some(" a".into());
        assert!(f.matches(&m));
        m.body = crate::MessageBody::Text("orderid".into());
        assert!(!f.matches(&m));
    }

    #[test]
    fn deserializes_from_partial_json() {
        let s: FilterSpec = serde_json::from_str(r#"{"property":"x = 1"}"#).unwrap();
        assert_eq!(s.property.as_deref(), Some("x = 1"));
        assert!(s.session_id.is_none());
    }

    #[test]
    fn regex_and_expected_value_keep_their_spaces() {
        let s = FilterSpec {
            body_regex: Some(" a b ".into()),
            ..Default::default()
        };
        let f = s.build().unwrap();
        let mut m = crate::messaging::test_message(1);
        m.body = crate::MessageBody::Text("xa bx".into());
        assert!(
            !f.matches(&m),
            "the surrounding spaces are part of the pattern"
        );
        m.body = crate::MessageBody::Text("x a b x".into());
        assert!(f.matches(&m));

        let s = FilterSpec {
            body_json_path: Some("$.s".into()),
            body_json_equals: Some(" shipped".into()),
            ..Default::default()
        };
        let f = s.build().unwrap();
        m.body = crate::MessageBody::Text(r#"{"s":" shipped"}"#.into());
        assert!(f.matches(&m));
        m.body = crate::MessageBody::Text(r#"{"s":"shipped"}"#.into());
        assert!(!f.matches(&m));
    }
}
