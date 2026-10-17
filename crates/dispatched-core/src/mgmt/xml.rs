//! A tiny lenient XML tree keyed by local names. ATOM responses vary in
//! namespace prefixes, so matching on local names is the robust choice.

use crate::{Error, Result};
use quick_xml::Reader;
use quick_xml::events::Event;

/// An element: local name, concatenated text, children.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Node {
    /// Local name (no prefix).
    pub name: String,
    /// Text content (trimmed, unescaped).
    pub text: String,
    /// Child elements in document order.
    pub children: Vec<Node>,
}

impl Node {
    /// First child with local name `name`.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// All children with local name `name`.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Descend through `path` of local names.
    pub fn path(&self, path: &[&str]) -> Option<&Node> {
        path.iter().try_fold(self, |node, name| node.child(name))
    }

    /// Non-empty text of child `name`.
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name)
            .map(|n| n.text.as_str())
            .filter(|t| !t.is_empty())
    }
}

/// Parse `xml` into its root element, trimming text.
pub fn parse(xml: &str) -> Result<Node> {
    parse_with(xml, true)
}

/// Parse `xml` keeping the exact text of leaf elements; only whitespace between child elements
/// is dropped.
pub fn parse_exact(xml: &str) -> Result<Node> {
    parse_with(xml, false)
}

fn parse_with(xml: &str, trim: bool) -> Result<Node> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(trim);
    let mut stack = vec![Node {
        name: "#document".into(),
        ..Node::default()
    }];
    loop {
        match reader
            .read_event()
            .map_err(|e| Error::Protocol(format!("invalid XML: {e}")))?
        {
            Event::Start(e) => stack.push(Node {
                name: String::from_utf8_lossy(e.local_name().as_ref()).into_owned(),
                ..Node::default()
            }),
            Event::Empty(e) => {
                let node = Node {
                    name: String::from_utf8_lossy(e.local_name().as_ref()).into_owned(),
                    ..Node::default()
                };
                stack.last_mut().expect("document node").children.push(node);
            }
            Event::Text(t) => {
                let text = t
                    .unescape()
                    .map_err(|e| Error::Protocol(format!("invalid XML text: {e}")))?;
                stack
                    .last_mut()
                    .expect("document node")
                    .text
                    .push_str(&text);
            }
            Event::CData(c) => {
                stack
                    .last_mut()
                    .expect("document node")
                    .text
                    .push_str(&String::from_utf8_lossy(&c));
            }
            Event::End(_) => {
                let mut node = stack.pop().expect("balanced end");
                if !trim && !node.children.is_empty() {
                    node.text = node.text.trim().to_string();
                }
                stack
                    .last_mut()
                    .ok_or_else(|| Error::Protocol("unbalanced XML".into()))?
                    .children
                    .push(node);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if stack.len() != 1 {
        return Err(Error::Protocol("truncated XML document".into()));
    }
    stack
        .pop()
        .and_then(|doc| doc.children.into_iter().next())
        .ok_or_else(|| Error::Protocol("empty XML document".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_names_ignore_prefixes() {
        let root = parse(r#"<a xmlns:p="x"><p:b>1</p:b><c/></a>"#).unwrap();
        assert_eq!(root.name, "a");
        assert_eq!(root.text_of("b"), Some("1"));
        assert!(root.child("c").is_some());
    }

    #[test]
    fn truncated_is_an_error() {
        assert!(parse("<a><b>").is_err());
    }
}
