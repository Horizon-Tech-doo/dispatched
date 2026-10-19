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
    /// The namespace prefix the element was written with (`d2p1` for `d2p1:Count`), so a
    /// writer can put an element it does not model back as it came.
    pub prefix: Option<String>,
    /// Attributes as written, qualified names included (`i:type`, `xmlns:d6p1`).
    pub attrs: Vec<(String, String)>,
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

    /// The value of the attribute whose local name is `local` (`type` finds `i:type`).
    /// Namespace declarations are not attributes here.
    pub fn attr(&self, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| {
                k != "xmlns" && !k.starts_with("xmlns:") && k.rsplit(':').next() == Some(local)
            })
            .map(|(_, v)| v.as_str())
    }

    /// The `i:type` of the element without its prefix (`d6p1:string` gives `string`).
    pub fn xsi_type(&self) -> Option<&str> {
        self.attr("type").map(|t| t.rsplit(':').next().unwrap_or(t))
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
            Event::Start(e) => stack.push(element(&e)?),
            Event::Empty(e) => {
                let node = element(&e)?;
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

fn element(e: &quick_xml::events::BytesStart<'_>) -> Result<Node> {
    let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
    let prefix = e
        .name()
        .prefix()
        .map(|p| String::from_utf8_lossy(p.as_ref()).into_owned());
    let mut attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|e| Error::Protocol(format!("invalid XML attribute: {e}")))?;
        let value = a
            .unescape_value()
            .map_err(|e| Error::Protocol(format!("invalid XML attribute: {e}")))?;
        attrs.push((
            String::from_utf8_lossy(a.key.as_ref()).into_owned(),
            value.into_owned(),
        ));
    }
    Ok(Node {
        name,
        prefix,
        attrs,
        ..Node::default()
    })
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
    fn attributes_and_prefixes_are_kept() {
        let root = parse(
            r#"<a xmlns:i="x" xmlns:d6p1="y"><Value i:type="d6p1:string">v</Value><d6p1:b/></a>"#,
        )
        .unwrap();
        let v = root.child("Value").unwrap();
        assert_eq!(v.attr("type"), Some("d6p1:string"));
        assert_eq!(v.xsi_type(), Some("string"));
        assert_eq!(
            root.attr("i"),
            None,
            "namespace declarations are not attributes"
        );
        assert_eq!(root.child("b").unwrap().prefix.as_deref(), Some("d6p1"));
    }

    #[test]
    fn truncated_is_an_error() {
        assert!(parse("<a><b>").is_err());
    }
}
