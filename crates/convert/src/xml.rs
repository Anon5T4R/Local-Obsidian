//! The smallest XML tree these two formats need.
//!
//! An office document is machine-generated XML whose whole point is that a reader can walk it
//! element by element; a tree is what makes that walk readable. `quick-xml` is the pull parser underneath
//! (pure Rust, no C), so entities, CDATA and attribute quoting are its problem, not ours.

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::Error;

/// One node of the tree: an element or the text between elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    Text(String),
}

/// An element with its attributes, the names as they appear in the document (`office:text`,
/// `w:p`) - both formats are namespaced and the prefixes are part of the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Element {
    /// The value of an attribute, by its full name (`text:outline-level`).
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }

    /// The element's own text, with descendants flattened in order.
    pub fn text(&self) -> String {
        let mut out = String::new();
        self.collect_text(&mut out);
        out
    }

    fn collect_text(&self, out: &mut String) {
        for child in &self.children {
            match child {
                Node::Text(text) => out.push_str(text),
                Node::Element(element) => element.collect_text(out),
            }
        }
    }

    /// The first descendant with this name, depth-first (the upstream's `find`).
    pub fn find(&self, name: &str) -> Option<&Element> {
        for child in self.elements() {
            if child.name == name {
                return Some(child);
            }
            if let Some(found) = child.find(name) {
                return Some(found);
            }
        }
        None
    }

    /// The direct child elements, in document order.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|child| match child {
            Node::Element(element) => Some(element),
            Node::Text(_) => None,
        })
    }
}

/// Parses a document into a tree rooted at a synthetic `#root` element, so a caller never has to
/// unwrap the first child. A file with more than one top-level element (or none) is still a tree.
pub fn parse(xml: &str) -> Result<Element, Error> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;

    let mut root = Element { name: "#root".to_string(), attrs: Vec::new(), children: Vec::new() };
    // The open elements, innermost last; a stray closing tag never pops past the root.
    let mut stack: Vec<Element> = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => stack.push(element(&start)?),
            Ok(Event::Empty(empty)) => {
                let done = element(&empty)?;
                push(&mut stack, &mut root, done);
            }
            Ok(Event::End(_)) => {
                if let Some(done) = stack.pop() {
                    push(&mut stack, &mut root, done);
                }
            }
            Ok(Event::Text(text)) => {
                let value = decode(&text);
                push_text(&mut stack, &mut root, value);
            }
            Ok(Event::CData(data)) => {
                let value = String::from_utf8_lossy(&data).to_string();
                push_text(&mut stack, &mut root, value);
            }
            // An entity is its own event: `&amp;` is not part of the text around it.
            Ok(Event::GeneralRef(reference)) => {
                let value = entity(&reference);
                push_text(&mut stack, &mut root, value);
            }
            Ok(Event::Eof) => break,
            // Comments, declarations and doctypes carry nothing a note shows.
            Ok(_) => {}
            Err(error) => return Err(Error::Xml(format!("{error}"))),
        }
    }
    // Whatever is still open is attached where it belongs (a truncated file still converts).
    while let Some(done) = stack.pop() {
        push(&mut stack, &mut root, done);
    }
    Ok(root)
}

/// An element from a start or empty tag, with its attributes decoded.
fn element(tag: &quick_xml::events::BytesStart<'_>) -> Result<Element, Error> {
    let mut attrs = Vec::new();
    for attribute in tag.attributes() {
        let attribute = attribute.map_err(|error| Error::Xml(format!("{error}")))?;
        let key = String::from_utf8_lossy(attribute.key.as_ref()).to_string();
        attrs.push((key, decode(&attribute.value)));
    }
    Ok(Element { name: String::from_utf8_lossy(tag.name().as_ref()).to_string(), attrs, children: Vec::new() })
}

/// The text of a node, with its XML entities resolved. An entity this crate does not know stays as
/// written - showing `&foo;` beats dropping the text around it.
fn decode(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    quick_xml::escape::unescape(&text).map_or_else(|_| text.to_string(), |value| value.to_string())
}

/// An entity reference as text: `&#233;` and `&#xE9;` are characters, the five XML names are
/// themselves, and anything else stays as the author wrote it.
fn entity(reference: &quick_xml::events::BytesRef<'_>) -> String {
    if let Ok(Some(character)) = reference.resolve_char_ref() {
        return character.to_string();
    }
    let name = String::from_utf8_lossy(reference);
    match name.as_ref() {
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" => "'".to_string(),
        "nbsp" => "\u{a0}".to_string(),
        other => format!("&{other};"),
    }
}

/// Attaches `done` to its parent, or to the root when nothing is open.
fn push(stack: &mut [Element], root: &mut Element, done: Element) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(Node::Element(done)),
        None => root.children.push(Node::Element(done)),
    }
}

/// Appends text to the innermost open element (or the root).
fn push_text(stack: &mut [Element], root: &mut Element, value: String) {
    if value.is_empty() {
        return;
    }
    match stack.last_mut() {
        Some(parent) => parent.children.push(Node::Text(value)),
        None => root.children.push(Node::Text(value)),
    }
}
