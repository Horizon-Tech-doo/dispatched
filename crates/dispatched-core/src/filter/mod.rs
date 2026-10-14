//! Client-side filters applied to peeked messages.

mod body;
mod expr;
mod lexer;

pub use body::BodyFilter;
pub use expr::PropertyFilter;

use crate::messaging::PeekedMessage;

/// Property and body conditions combined with AND.
#[derive(Debug, Clone, Default)]
pub struct MessageFilter {
    /// Optional property expression.
    pub property: Option<PropertyFilter>,
    /// Body conditions (all must match).
    pub body: Vec<BodyFilter>,
}

impl MessageFilter {
    /// No conditions.
    pub fn is_empty(&self) -> bool {
        self.property.is_none() && self.body.is_empty()
    }

    /// Whether `msg` satisfies every condition.
    pub fn matches(&self, msg: &PeekedMessage) -> bool {
        self.property.as_ref().is_none_or(|p| p.matches(msg))
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
        };
        assert!(f.matches(&m));
        m.subject = Some("b".into());
        assert!(!f.matches(&m));
    }
}
