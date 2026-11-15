//! Word (`.docx`) into this app's Markdown.
//!
//! The upstream leaves this to mammoth + turndown (XML → HTML → Markdown). A note needs far less
//! than a full Word layout: headings, paragraphs, bold/italic, links, lists and simple tables cover
//! what a document in a vault actually contains, so this is a focused reader instead of a
//! converter-of-everything.
//!
//! Three parts of the package are read: `word/document.xml` (the body), `word/_rels/document.xml.rels`
//! (where a `w:hyperlink` points) and `word/numbering.xml` (whether a `numId` counts or bullets).
//! Pictures, charts, footnotes, comments and tracked changes are dropped, exactly as the upstream's
//! limitations.md #8 keeps them out of the text - an image without its file would only be a broken
//! reference in the note.

use std::collections::HashMap;

use crate::Error;
use crate::text::{escape, mark};
use crate::xml::{self, Element, Node};

/// The parts this format needs.
const DOCUMENT: &str = "word/document.xml";
const RELS: &str = "word/_rels/document.xml.rels";
const NUMBERING: &str = "word/numbering.xml";

/// The styles Word names its headings with, in the order a note uses them.
const HEADING_STYLES: [&str; 6] = ["Heading1", "Heading2", "Heading3", "Heading4", "Heading5", "Heading6"];

/// Reads the `.docx` package at `package` (the file's bytes) into Markdown.
pub fn from_package(package: &[u8]) -> Result<String, Error> {
    let document = crate::read_part(package, DOCUMENT)?.ok_or(Error::Missing(DOCUMENT))?;
    let rels = crate::read_part(package, RELS)?.unwrap_or_default();
    let numbering = crate::read_part(package, NUMBERING)?.unwrap_or_default();
    to_markdown(&document, &rels, &numbering)
}

/// The three parts as text. Each may be empty: a document with no links and no lists is normal.
pub fn to_markdown(document_xml: &str, rels_xml: &str, numbering_xml: &str) -> Result<String, Error> {
    let document = xml::parse(document_xml)?;
    let context = Context { links: link_targets(rels_xml)?, lists: list_kinds(numbering_xml)? };
    let Some(body) = document.find("w:body") else { return Ok(String::new()) };
    let lines = blocks(&body.children, &context);
    Ok(crate::join_document(&lines))
}

/// What a paragraph or run needs from the rest of the package.
struct Context {
    /// `rId` → where a hyperlink points.
    links: HashMap<String, String>,
    /// `numId` → true when the list counts rather than bullets.
    lists: HashMap<String, bool>,
}

/// `rId` → target, from the document's relationships part.
fn link_targets(rels_xml: &str) -> Result<HashMap<String, String>, Error> {
    let mut links = HashMap::new();
    if rels_xml.trim().is_empty() {
        return Ok(links);
    }
    let rels = xml::parse(rels_xml)?;
    for relationship in rels.find("Relationships").map(|root| root.elements()).into_iter().flatten() {
        if relationship.name != "Relationship" {
            continue;
        }
        if let (Some(id), Some(target)) = (relationship.attr("Id"), relationship.attr("Target")) {
            links.insert(id.to_string(), target.to_string());
        }
    }
    Ok(links)
}

/// `numId` → counted or bulleted, from `numbering.xml`.
///
/// The indirection is Word's: a `w:num` names an abstract list, and the abstract list's first level
/// says `decimal` or `bullet`. A `numId` that the part does not describe is a bullet, which is the
/// safer guess - a missing number in a note is invisible, a wrong one is not.
fn list_kinds(numbering_xml: &str) -> Result<HashMap<String, bool>, Error> {
    let mut lists = HashMap::new();
    if numbering_xml.trim().is_empty() {
        return Ok(lists);
    }
    let numbering = xml::parse(numbering_xml)?;

    let mut abstract_kinds: HashMap<String, bool> = HashMap::new();
    for abstract_list in numbering.find("w:numbering").map(|root| root.elements()).into_iter().flatten() {
        if abstract_list.name != "w:abstractNum" {
            continue;
        }
        let Some(id) = abstract_list.attr("w:abstractNumId") else { continue };
        let ordered = abstract_list
            .elements()
            .find(|child| child.name == "w:lvl")
            .and_then(|level| level.find("w:numFmt"))
            .and_then(|format| format.attr("w:val"))
            .is_some_and(|format| format != "bullet" && format != "none");
        abstract_kinds.insert(id.to_string(), ordered);
    }

    for list in numbering.find("w:numbering").map(|root| root.elements()).into_iter().flatten() {
        if list.name != "w:num" {
            continue;
        }
        let Some(id) = list.attr("w:numId") else { continue };
        let ordered = list
            .find("w:abstractNumId")
            .and_then(|reference| reference.attr("w:val"))
            .and_then(|abstract_id| abstract_kinds.get(abstract_id).copied())
            .unwrap_or(false);
        lists.insert(id.to_string(), ordered);
    }
    Ok(lists)
}

/// The body's blocks as lines. Consecutive items of one list become a single block, so their lines
/// are not separated by blank lines.
fn blocks(nodes: &[Node], context: &Context) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut open_list: Option<(String, Vec<String>)> = None;
    let mut number = 0;

    let flush = |open: &mut Option<(String, Vec<String>)>, out: &mut Vec<String>| {
        if let Some((_, lines)) = open.take()
            && !lines.is_empty()
        {
            out.push(lines.join("\n"));
        }
    };

    for node in flatten(nodes) {
        match node.name.as_str() {
            "w:p" => {
                if let Some(list) = list_marker(node, context) {
                    let text = inline_children(&node.children, context, RunStyle::default());
                    match &mut open_list {
                        Some((id, lines)) if *id == list.num_id => {
                            number += 1;
                            lines.push(format!("{}{} {text}", list.indent, list.marker(number)));
                        }
                        _ => {
                            flush(&mut open_list, &mut out);
                            number = 1;
                            open_list = Some((list.num_id.clone(), vec![format!("{}{} {text}", list.indent, list.marker(1))]));
                        }
                    }
                    continue;
                }
                flush(&mut open_list, &mut out);
                if let Some(line) = paragraph(node, context) {
                    out.push(line);
                }
            }
            "w:tbl" => {
                flush(&mut open_list, &mut out);
                let table = table(node, context);
                if !table.is_empty() {
                    out.push(table);
                }
            }
            // Page setup, bookmarks and the section break carry no text.
            _ => {
                flush(&mut open_list, &mut out);
            }
        }
    }
    flush(&mut open_list, &mut out);
    out
}

/// The children of a body, with Word's content wrappers (`w:sdt`) opened up: a control's content is
/// ordinary body content as far as a note is concerned.
fn flatten(nodes: &[Node]) -> Vec<&Element> {
    let mut out = Vec::new();
    for node in nodes {
        let Node::Element(element) = node else { continue };
        if element.name == "w:sdt" {
            for content in element.elements().filter(|child| child.name == "w:sdtContent") {
                out.extend(flatten(&content.children));
            }
            continue;
        }
        out.push(element);
    }
    out
}

/// A list item's marker data, when the paragraph belongs to a list.
struct ListMarker {
    num_id: String,
    /// Two spaces per nesting level, ready to prefix the line.
    indent: String,
    ordered: bool,
}

impl ListMarker {
    fn marker(&self, number: usize) -> String {
        if self.ordered { format!("{number}.") } else { "-".to_string() }
    }
}

/// Whether the paragraph is a list item, and at which level.
fn list_marker(paragraph: &Element, context: &Context) -> Option<ListMarker> {
    let properties = paragraph.elements().find(|child| child.name == "w:pPr")?;
    let numbering = properties.elements().find(|child| child.name == "w:numPr")?;
    let num_id = numbering.elements().find(|child| child.name == "w:numId").and_then(|id| id.attr("w:val")).unwrap_or_default().to_string();
    let level = numbering
        .elements()
        .find(|child| child.name == "w:ilvl")
        .and_then(|level| level.attr("w:val"))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
        .clamp(0, 8);
    Some(ListMarker { ordered: context.lists.get(&num_id).copied().unwrap_or(false), num_id, indent: "  ".repeat(level) })
}

/// A paragraph as a note line: a heading when Word says so, plain text otherwise.
fn paragraph(paragraph: &Element, context: &Context) -> Option<String> {
    let text = inline_children(&paragraph.children, context, RunStyle::default());
    if text.trim().is_empty() {
        return None;
    }
    match heading_level(paragraph) {
        Some(level) => Some(format!("{} {text}", "#".repeat(level))),
        None => Some(text),
    }
}

/// The heading level a paragraph's style or outline level gives it.
fn heading_level(paragraph: &Element) -> Option<usize> {
    let properties = paragraph.elements().find(|child| child.name == "w:pPr")?;
    if let Some(style) = properties.elements().find(|child| child.name == "w:pStyle").and_then(|style| style.attr("w:val")) {
        // `Heading1`..`Heading6`, and Word's own Portuguese and Spanish names, which are the ones a
        // document written here carries.
        let normalized = style.replace(' ', "").to_lowercase();
        for (index, name) in HEADING_STYLES.iter().enumerate() {
            if normalized == name.to_lowercase() {
                return Some(index + 1);
            }
        }
        for (prefix, fallback) in [("titulo", 1usize), ("título", 1), ("title", 1)] {
            if let Some(rest) = normalized.strip_prefix(prefix) {
                let level = rest.parse::<usize>().unwrap_or(fallback);
                return Some(level.clamp(1, 6));
            }
        }
    }
    properties
        .elements()
        .find(|child| child.name == "w:outlineLvl")
        .and_then(|level| level.attr("w:val"))
        .and_then(|value| value.parse::<usize>().ok())
        .map(|value| (value + 1).clamp(1, 6))
}

/// How a run of text is drawn: the emphasis carried down from the paragraph to the run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RunStyle {
    bold: bool,
    italic: bool,
}

/// The inline content of a paragraph, a hyperlink or a cell.
fn inline_children(nodes: &[Node], context: &Context, style: RunStyle) -> String {
    let mut out = String::new();
    for node in nodes {
        let Node::Element(element) = node else {
            if let Node::Text(text) = node {
                // Text directly under a paragraph is unusual but still text.
                out.push_str(&escape(text));
            }
            continue;
        };
        match element.name.as_str() {
            "w:r" => out.push_str(&run(element, style)),
            "w:hyperlink" => {
                let text = inline_children(&element.children, context, style);
                let target = element
                    .attr("r:id")
                    .and_then(|id| context.links.get(id))
                    .cloned()
                    .or_else(|| element.attr("w:anchor").map(|anchor| format!("#{anchor}")));
                match target {
                    Some(target) if !text.trim().is_empty() => out.push_str(&format!("[{text}]({})", target.replace(' ', "%20"))),
                    _ => out.push_str(&text),
                }
            }
            // A tracked change's inserted text is what the document says; the deleted one is gone.
            "w:ins" | "w:smartTag" | "w:customXml" | "w:sdt" => out.push_str(&inline_children(&element.children, context, style)),
            "w:sdtContent" => out.push_str(&inline_children(&element.children, context, style)),
            _ => {}
        }
    }
    out
}

/// One run: its text, with the emphasis its properties ask for.
fn run(run: &Element, inherited: RunStyle) -> String {
    let style = run_style(run, inherited);
    let mut out = String::new();
    for child in &run.children {
        let Node::Element(element) = child else {
            if let Node::Text(text) = child {
                out.push_str(&escape(text));
            }
            continue;
        };
        match element.name.as_str() {
            "w:t" => out.push_str(&escape(&element.text())),
            "w:br" | "w:cr" => out.push_str("  \n"),
            "w:tab" => out.push(' '),
            // A picture, a field result or a footnote reference: nothing a note can hold.
            _ => {}
        }
    }
    mark(&out, style.bold, style.italic)
}

/// The emphasis a run adds to what it inherits.
fn run_style(run: &Element, inherited: RunStyle) -> RunStyle {
    let Some(properties) = run.elements().find(|child| child.name == "w:rPr") else { return inherited };
    let toggle = |name: &str, inherited: bool| match properties.elements().find(|child| child.name == name) {
        // `<w:b/>` is on; `<w:b w:val="0"/>` is explicitly off.
        Some(flag) => !matches!(flag.attr("w:val"), Some("0") | Some("false")),
        None => inherited,
    };
    RunStyle { bold: toggle("w:b", inherited.bold), italic: toggle("w:i", inherited.italic) }
}

/// A table as a GFM table, the first row as the header (what mammoth + turndown-gfm produce too).
fn table(table: &Element, context: &Context) -> String {
    let rows: Vec<Vec<String>> = table
        .elements()
        .filter(|child| child.name == "w:tr")
        .map(|row| {
            row.elements()
                .filter(|cell| cell.name == "w:tc")
                .map(|cell| {
                    // A cell holds paragraphs; a note's cell is one line, so they join with a space.
                    let text: Vec<String> = cell
                        .elements()
                        .filter(|child| child.name == "w:p")
                        .map(|paragraph| inline_children(&paragraph.children, context, RunStyle::default()))
                        .filter(|text| !text.trim().is_empty())
                        .collect();
                    text.join(" ").replace('|', "\\|")
                })
                .collect()
        })
        .filter(|row: &Vec<String>| !row.is_empty())
        .collect();
    let Some(first) = rows.first() else { return String::new() };

    let width = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let line = |cells: &[String]| {
        let mut cells = cells.to_vec();
        cells.resize(width, String::new());
        format!("| {} |", cells.join(" | "))
    };
    let mut out = vec![line(first), format!("| {} |", vec!["---"; width].join(" | "))];
    out.extend(rows.iter().skip(1).map(|cells| line(cells)));
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(body: &str) -> String {
        format!("<w:document xmlns:w=\"w\"><w:body>{body}</w:body></w:document>")
    }

    fn convert(body: &str) -> String {
        to_markdown(&doc(body), "", "").expect("the document converts")
    }

    fn paragraph(text: &str) -> String {
        format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>")
    }

    #[test]
    fn paragraphs_become_blocks() {
        assert_eq!(convert(&format!("{}{}", paragraph("Um texto"), paragraph("Outro"))), "Um texto\n\nOutro\n");
    }

    #[test]
    fn an_empty_paragraph_is_dropped() {
        assert_eq!(convert(&format!("{}<w:p/>{}/", paragraph("x"), "")), "x\n");
    }

    #[test]
    fn a_heading_style_becomes_a_heading() {
        let heading = "<w:p><w:pPr><w:pStyle w:val=\"Heading2\"/></w:pPr><w:r><w:t>Conduta</w:t></w:r></w:p>";
        assert_eq!(convert(heading), "## Conduta\n");
    }

    #[test]
    fn a_localized_heading_style_is_read_too() {
        let heading = "<w:p><w:pPr><w:pStyle w:val=\"Titulo1\"/></w:pPr><w:r><w:t>Conduta</w:t></w:r></w:p>";
        assert_eq!(convert(heading), "# Conduta\n");
    }

    #[test]
    fn an_outline_level_also_makes_a_heading() {
        let heading = "<w:p><w:pPr><w:outlineLvl w:val=\"2\"/></w:pPr><w:r><w:t>Seção</w:t></w:r></w:p>";
        assert_eq!(convert(heading), "### Seção\n");
    }

    #[test]
    fn bold_and_italic_come_from_the_run_properties() {
        let body =
            "<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>forte</w:t></w:r><w:r><w:t> e </w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>ênfase</w:t></w:r></w:p>";
        assert_eq!(convert(body), "**forte** e *ênfase*\n");
    }

    #[test]
    fn a_run_can_turn_emphasis_off_again() {
        let body = "<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>forte</w:t></w:r><w:r><w:rPr><w:b w:val=\"0\"/></w:rPr><w:t> normal</w:t></w:r></w:p>";
        assert_eq!(convert(body), "**forte** normal\n");
    }

    #[test]
    fn a_hyperlink_keeps_its_target() {
        let rels = "<Relationships><Relationship Id=\"rId5\" Type=\"hyperlink\" Target=\"https://x.com\"/></Relationships>";
        let body = "<w:p><w:hyperlink r:id=\"rId5\"><w:r><w:t>site</w:t></w:r></w:hyperlink></w:p>";
        assert_eq!(to_markdown(&doc(body), rels, "").expect("valid"), "[site](https://x.com)\n");
    }

    #[test]
    fn a_hyperlink_to_a_bookmark_stays_inside_the_document() {
        let body = "<w:p><w:hyperlink w:anchor=\"topo\"><w:r><w:t>voltar</w:t></w:r></w:hyperlink></w:p>";
        assert_eq!(convert(body), "[voltar](#topo)\n");
    }

    #[test]
    fn a_bulleted_list_is_one_block() {
        let numbering = "<w:numbering><w:abstractNum w:abstractNumId=\"0\"><w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"bullet\"/></w:lvl></w:abstractNum><w:num w:numId=\"2\"><w:abstractNumId w:val=\"0\"/></w:num></w:numbering>";
        let item = |text: &str| {
            format!("<w:p><w:pPr><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"2\"/></w:numPr></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>")
        };
        let body = format!("{}{}", item("um"), item("dois"));
        assert_eq!(to_markdown(&doc(&body), "", numbering).expect("valid"), "- um\n- dois\n");
    }

    #[test]
    fn a_numbered_list_counts() {
        let numbering = "<w:numbering><w:abstractNum w:abstractNumId=\"1\"><w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"decimal\"/></w:lvl></w:abstractNum><w:num w:numId=\"3\"><w:abstractNumId w:val=\"1\"/></w:num></w:numbering>";
        let item = |text: &str| {
            format!("<w:p><w:pPr><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"3\"/></w:numPr></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>")
        };
        let body = format!("{}{}", item("um"), item("dois"));
        assert_eq!(to_markdown(&doc(&body), "", numbering).expect("valid"), "1. um\n2. dois\n");
    }

    #[test]
    fn a_nested_list_level_is_indented() {
        let numbering = "<w:numbering><w:abstractNum w:abstractNumId=\"0\"><w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"bullet\"/></w:lvl></w:abstractNum><w:num w:numId=\"2\"><w:abstractNumId w:val=\"0\"/></w:num></w:numbering>";
        let item = |text: &str, level: usize| {
            format!("<w:p><w:pPr><w:numPr><w:ilvl w:val=\"{level}\"/><w:numId w:val=\"2\"/></w:numPr></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>")
        };
        let body = format!("{}{}", item("pai", 0), item("filho", 1));
        assert_eq!(to_markdown(&doc(&body), "", numbering).expect("valid"), "- pai\n  - filho\n");
    }

    #[test]
    fn a_list_ends_when_a_paragraph_starts() {
        let numbering = "<w:numbering><w:abstractNum w:abstractNumId=\"0\"><w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"bullet\"/></w:lvl></w:abstractNum><w:num w:numId=\"2\"><w:abstractNumId w:val=\"0\"/></w:num></w:numbering>";
        let item = "<w:p><w:pPr><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"2\"/></w:numPr></w:pPr><w:r><w:t>um</w:t></w:r></w:p>";
        let body = format!("{item}{}", paragraph("depois"));
        assert_eq!(to_markdown(&doc(&body), "", numbering).expect("valid"), "- um\n\ndepois\n");
    }

    #[test]
    fn a_table_uses_its_first_row_as_the_header() {
        let row = |a: &str, b: &str| format!("<w:tr><w:tc>{}</w:tc><w:tc>{}</w:tc></w:tr>", paragraph(a), paragraph(b));
        let body = format!("<w:tbl>{}{}</w:tbl>", row("Droga", "Dose"), row("Nora", "0,1"));
        assert_eq!(convert(&body), "| Droga | Dose |\n| --- | --- |\n| Nora | 0,1 |\n");
    }

    #[test]
    fn a_pipe_inside_a_cell_does_not_break_the_table() {
        let row = |a: &str| format!("<w:tr><w:tc>{}</w:tc></w:tr>", paragraph(a));
        let body = format!("<w:tbl>{}{}</w:tbl>", row("a|b"), row("c"));
        assert_eq!(convert(&body), "| a\\|b |\n| --- |\n| c |\n");
    }

    #[test]
    fn a_line_break_stays_a_line_break() {
        let body = "<w:p><w:r><w:t>a</w:t><w:br/><w:t>b</w:t></w:r></w:p>";
        assert_eq!(convert(body), "a  \nb\n");
    }

    #[test]
    fn pictures_are_dropped_and_the_text_around_them_kept() {
        let body = "<w:p><w:r><w:drawing><w:blip r:embed=\"rId9\"/></w:drawing></w:r><w:r><w:t>legenda</w:t></w:r></w:p>";
        assert_eq!(convert(body), "legenda\n");
    }

    #[test]
    fn a_document_without_a_body_is_empty() {
        assert_eq!(to_markdown("<w:document></w:document>", "", "").expect("valid"), "");
    }

    #[test]
    fn text_that_looks_like_markup_is_escaped() {
        assert_eq!(convert(&paragraph("# não é título")), "\\# não é título\n");
        assert_eq!(convert(&paragraph("a*b")), "a\\*b\n");
    }
}
