//! Client-side filters applied to peeked messages.

mod body;
mod expr;
mod lexer;
mod spec;

pub use body::BodyFilter;
pub use expr::PropertyFilter;
pub use spec::{FilterField, FilterIssue, FilterSpec};

use crate::messaging::PeekedMessage;

/// Property, body and session conditions combined with AND.
#[derive(Debug, Clone, Default)]
pub struct MessageFilter {
    /// Optional property expression.
    pub property: Option<PropertyFilter>,
    /// Body conditions (all must match).
    pub body: Vec<BodyFilter>,
    /// Only messages of this session (exact comparison, as Service Bus compares session ids).
    pub session_id: Option<String>,
}

impl MessageFilter {
    /// No conditions.
    pub fn is_empty(&self) -> bool {
        self.property.is_none() && self.body.is_empty() && self.session_id.is_none()
    }

    /// Whether `msg` satisfies every condition.
    pub fn matches(&self, msg: &PeekedMessage) -> bool {
        self.session_id
            .as_deref()
            .is_none_or(|s| msg.session_id.as_deref() == Some(s))
            && self.property.as_ref().is_none_or(|p| p.matches(msg))
            && self.body.iter().all(|b| b.matches(&msg.body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::{MessageBody, test_message};

    #[test]
    fn empty_filter_matches_everything() {
        assert!(MessageFilter::default().is_empty());
        assert!(MessageFilter::default().matches(&test_message(1)));
    }

    #[test]
    fn property_and_body_are_anded() {
        let mut m = test_message(1);
        m.subject = Some("a".into());
        m.body = MessageBody::Text("hello world".into());
        let f = MessageFilter {
            property: Some(PropertyFilter::parse("sys.Subject = 'a'").unwrap()),
            body: vec![BodyFilter::contains("WORLD")],
            ..Default::default()
        };
        assert!(f.matches(&m));
        m.subject = Some("b".into());
        assert!(!f.matches(&m));
    }

    #[test]
    fn session_condition_is_exact() {
        let mut m = test_message(1);
        m.session_id = Some("RMA-1001".into());
        let f = MessageFilter {
            session_id: Some("RMA-1001".into()),
            ..Default::default()
        };
        assert!(!f.is_empty());
        assert!(f.matches(&m));
        m.session_id = Some("rma-1001".into());
        assert!(!f.matches(&m), "session ids compare exactly");
        m.session_id = None;
        assert!(!f.matches(&m));
    }
}
