//! Minimal XML DOM: eSCL responses mix the `scan:` and `pwg:` prefixes,
//! so only local names are kept.

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

use crate::i18n::i18n;
use crate::{Error, Result};

#[derive(Debug, Default, Clone)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    pub text: String,
}

impl Element {
    pub fn parse(xml: &str) -> Result<Element> {
        // No `trim_text`: it would cut the spaces around entities (`1 &amp; 2`).
        let mut reader = Reader::from_str(xml);
        // The first stack element holds the root node.
        let mut stack = vec![Element::default()];
        loop {
            match reader.read_event().map_err(|e| Error::Xml(e.to_string()))? {
                Event::Start(start) => stack.push(Element::from_start(&start)?),
                Event::Empty(start) => {
                    let element = Element::from_start(&start)?;
                    stack.last_mut().expect("non-empty stack").children.push(element);
                }
                Event::End(_) => {
                    let element = stack.pop().expect("non-empty stack");
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(element),
                        None => return Err(Error::Xml(i18n("unexpected closing tag").into())),
                    }
                }
                Event::Text(text) => {
                    if let Some(current) = stack.last_mut() {
                        current.text.push_str(&text.xml_content(XmlVersion::Implicit1_0));
                    }
                }
                Event::CData(data) => {
                    if let Some(current) = stack.last_mut() {
                        current.text.push_str(&data.xml_content(XmlVersion::Implicit1_0));
                    }
                }
                Event::GeneralRef(reference) => {
                    if let Some(current) = stack.last_mut() {
                        let resolved = match reference.resolve_char_ref() {
                            Ok(Some(c)) => Some(c),
                            _ => match reference.into_inner().as_ref() {
                                "amp" => Some('&'),
                                "lt" => Some('<'),
                                "gt" => Some('>'),
                                "quot" => Some('"'),
                                "apos" => Some('\''),
                                _ => None,
                            },
                        };
                        if let Some(c) = resolved {
                            current.text.push(c);
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
        if stack.len() != 1 {
            return Err(Error::Xml(i18n("truncated XML document").into()));
        }
        stack
            .pop()
            .and_then(|holder| holder.children.into_iter().next())
            .ok_or_else(|| Error::Xml(i18n("empty XML document").into()))
    }

    fn from_start(start: &BytesStart) -> Result<Element> {
        let mut attrs = Vec::new();
        for attr in start.attributes() {
            let attr = attr.map_err(|e| Error::Xml(e.to_string()))?;
            let value = attr.normalized_value(XmlVersion::Implicit1_0).map_err(|e| Error::Xml(e.to_string()))?;
            attrs.push((attr.key.local_name().as_ref().to_owned(), value.into_owned()));
        }
        Ok(Element { name: start.local_name().as_ref().to_owned(), attrs, ..Default::default() })
    }

    /// First direct child with this local name.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Direct children with this local name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Follows a path of direct children.
    pub fn path(&self, path: &[&str]) -> Option<&Element> {
        path.iter().try_fold(self, |el, name| el.child(name))
    }

    /// First descendant (depth-first) with this local name.
    pub fn descendant(&self, name: &str) -> Option<&Element> {
        self.children.iter().find_map(|c| if c.name == name { Some(c) } else { c.descendant(name) })
    }

    /// All descendants with this local name.
    pub fn descendants<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        for child in &self.children {
            if child.name == name {
                out.push(child);
            }
            child.descendants(name, out);
        }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> &str {
        self.text.trim()
    }

    pub fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name).map(Element::text)
    }

    pub fn child_u32(&self, name: &str) -> Option<u32> {
        self.child_text(name).and_then(|t| t.parse().ok())
    }

    pub fn child_i32(&self, name: &str) -> Option<i32> {
        self.child_text(name).and_then(|t| t.parse().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_names_and_text() {
        let xml = r#"<?xml version="1.0"?><!----><a:Root xmlns:a="x"><b:Item b:default="true">1 &amp; 2</b:Item><Empty/></a:Root>"#;
        let root = Element::parse(xml).unwrap();
        assert_eq!(root.name, "Root");
        let item = root.child("Item").unwrap();
        assert_eq!(item.text(), "1 & 2");
        assert_eq!(item.attr("default"), Some("true"));
        assert!(root.child("Empty").is_some());
    }
}
