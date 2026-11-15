//! OpenDocument Text (`.odt`) into this app's Markdown.
//!
//! The shape of an `.odt` is fixed: `content.xml` is the body, `styles.xml` holds the styles the
//! body refers to by name, and the text styles carry bold/italic/underline as attributes. The port
//! reads both parts the way the upstream does - its `content.xml` → HTML step is skipped, and the
//! blocks below are written straight as Markdown.
//!
//! The one thing ODF has and Markdown does not is underline: a styled span keeps its text and
//! loses the underline, which is the same trade every Markdown exporter makes.

use std::collections::{HashMap, HashSet};

use crate::Error;
use crate::text::{escape, mark};
use crate::xml::{self, Element, Node};

/// The parts of the package this format needs, in the order they are read.
const CONTENT: &str = "content.xml";
const STYLES: &str = "styles.xml";

/// Elements that never become note text: declarations, page furniture, and the objects the
/// upstream drops as well (pictures, forms, comments, tracked changes).
const SKIPPED: [&str; 6] = ["text:sequence-decls", "text:soft-page-break", "office:forms", "text:tracked-changes", "draw:frame", "office:annotation"];

/// Reads the `.odt` package at `package` (the file's bytes) into Markdown.
pub fn from_package(package: &[u8]) -> Result<String, Error> {
    let content = crate::read_part(package, CONTENT)?.ok_or(Error::Missing(CONTENT))?;
    let styles = crate::read_part(package, STYLES)?.unwrap_or_default();
    to_markdown(&content, &styles)
}

/// `content.xml` (and `styles.xml` when the body reuses a style from it) into Markdown.
pub fn to_markdown(content_xml: &str, styles_xml: &str) -> Result<String, Error> {
    let content = xml::parse(content_xml)?;
    let mut styles = Styles::collect(&content);
    if !styles_xml.is_empty() {
        styles.merge(&Styles::collect(&xml::parse(styles_xml)?));
    }
    // `office:text` is the body every `.odt` has; without it there is nothing to convert.
    let Some(body) = content.find("office:text") else { return Ok(String::new()) };
    let lines = blocks(&body.children, &styles, 0);
    Ok(crate::join_document(&lines))
}

/// A text style's inline formatting, read from the document's own style definitions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TextStyle {
    bold: bool,
    italic: bool,
    underline: bool,
}

/// What the document declares: the text styles and which list styles count from one.
#[derive(Debug, Default)]
struct Styles {
    text: HashMap<String, TextStyle>,
    ordered: HashSet<String>,
}

impl Styles {
    fn collect(root: &Element) -> Self {
        let mut styles = Styles::default();
        styles.walk(root);
        styles
    }

    fn walk(&mut self, node: &Element) {
        for child in node.elements() {
            if child.name == "style:style"
                && child.attr("style:family") == Some("text")
                && let (Some(name), Some(properties)) = (child.attr("style:name"), child.find("style:text-properties"))
            {
                self.text.insert(
                    name.to_string(),
                    TextStyle {
                        bold: properties.attr("fo:font-weight") == Some("bold"),
                        italic: properties.attr("fo:font-style") == Some("italic"),
                        underline: properties.attr("style:text-underline-style").is_some_and(|value| value != "none"),
                    },
                );
            }
            if child.name == "text:list-style"
                && let Some(name) = child.attr("style:name")
                && child.elements().next().is_some_and(|first| first.name == "text:list-level-style-number")
            {
                self.ordered.insert(name.to_string());
            }
            self.walk(child);
        }
    }

    /// The styles of a second part (`styles.xml`): what the body already defined wins.
    fn merge(&mut self, other: &Styles) {
        for (name, style) in other.text.clone() {
            self.text.entry(name).or_insert(style);
        }
        for name in other.ordered.clone() {
            self.ordered.insert(name);
        }
    }
}

/// The document's blocks as lines. Lists and their nested lists are indented here, so a caller only
/// has to join the lines with newlines.
fn blocks(nodes: &[Node], styles: &Styles, depth: usize) -> Vec<String> {
    let mut out = Vec::new();
    for node in nodes {
        let Node::Element(element) = node else { continue };
        if SKIPPED.contains(&element.name.as_str()) {
            continue;
        }
        match element.name.as_str() {
            "text:h" => {
                let level = element.attr("text:outline-level").and_then(|value| value.parse::<usize>().ok()).unwrap_or(1).clamp(1, 6);
                let text = inline(&element.children, styles).trim().to_string();
                if !text.is_empty() {
                    out.push(format!("{} {text}", "#".repeat(level)));
                }
            }
            "text:p" => {
                let text = inline(&element.children, styles).trim().to_string();
                if !text.is_empty() {
                    out.push(text);
                }
            }
            "text:list" => out.push(list(element, styles, depth).join("\n")),
            "table:table" => {
                let table = table(element, styles);
                if !table.is_empty() {
                    out.push(table);
                }
            }
            // A section, a header block or any wrapper simply passes its children up.
            _ => out.extend(blocks(&element.children, styles, depth)),
        }
    }
    out
}

/// One list, its items indented under `depth`.
fn list(element: &Element, styles: &Styles, depth: usize) -> Vec<String> {
    let ordered = styles.ordered.contains(element.attr("text:style-name").unwrap_or_default());
    let indent = "  ".repeat(depth);
    let mut out = Vec::new();
    let mut number = 0;
    for item in element.elements().filter(|child| child.name == "text:list-item" || child.name == "text:list-header") {
        number += 1;
        let marker = if ordered { format!("{number}.") } else { "-".to_string() };
        let inner = blocks(&item.children, styles, depth + 1);
        let mut lines = inner.into_iter();
        match lines.next() {
            // A list item is almost always one paragraph: its text sits on the marker's line.
            Some(first) => out.push(format!("{indent}{marker} {first}")),
            None => out.push(format!("{indent}{marker}")),
        }
        // Whatever else the item held - a nested list, a table - keeps its own indentation.
        out.extend(lines);
    }
    out
}

/// A table as a GFM table: the rows the document marks as header win, else the first row is one.
fn table(element: &Element, styles: &Styles) -> String {
    let mut rows: Vec<(Vec<String>, bool)> = Vec::new();
    collect_rows(element, false, styles, &mut rows);
    if rows.is_empty() {
        return String::new();
    }

    // The header is the first row the document flags, or the first row at all.
    let header_index = rows.iter().position(|(_, header)| *header).unwrap_or(0);
    let width = rows.iter().map(|(cells, _)| cells.len()).max().unwrap_or(0).max(1);
    let pad = |cells: &[String]| {
        let mut cells = cells.to_vec();
        cells.resize(width, String::new());
        format!("| {} |", cells.join(" | "))
    };

    let mut out = vec![pad(&rows[header_index].0), format!("| {} |", vec!["---"; width].join(" | "))];
    for (index, (cells, _)) in rows.iter().enumerate() {
        if index != header_index {
            out.push(pad(cells));
        }
    }
    out.join("\n")
}

/// Walks the rows of a table, unwrapping the header/row-group containers, and flags the header ones.
fn collect_rows(node: &Element, in_header: bool, styles: &Styles, rows: &mut Vec<(Vec<String>, bool)>) {
    for child in node.elements() {
        match child.name.as_str() {
            "table:table-header-rows" => collect_rows(child, true, styles, rows),
            "table:table-row-group" | "table:table-rows" => collect_rows(child, in_header, styles, rows),
            "table:table-row" => rows.push((cells(child, styles), in_header)),
            _ => {}
        }
    }
}

/// A row's cells, honouring `table:number-columns-repeated`.
fn cells(row: &Element, styles: &Styles) -> Vec<String> {
    let mut out = Vec::new();
    for cell in row.elements().filter(|child| child.name == "table:table-cell" || child.name == "table:covered-table-cell") {
        let repeat = cell.attr("table:number-columns-repeated").and_then(|value| value.parse::<usize>().ok()).unwrap_or(1).clamp(1, 64);
        let text = blocks(&cell.children, styles, 0).join(" ").replace('|', "\\|");
        for _ in 0..repeat {
            out.push(text.clone());
        }
    }
    out
}

/// A run of inline content, its emphasis and links kept.
fn inline(nodes: &[Node], styles: &Styles) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            Node::Text(text) => out.push_str(&escape(&text.replace(['\n', '\r'], " "))),
            Node::Element(element) => out.push_str(&inline_element(element, styles)),
        }
    }
    out
}

fn inline_element(element: &Element, styles: &Styles) -> String {
    match element.name.as_str() {
        "text:span" => {
            let inner = inline(&element.children, styles);
            let style = style_of(element, styles);
            // Markdown has no underline; the text stays, the line does not.
            mark(&inner, style.bold, style.italic)
        }
        "text:a" => {
            let href = element.attr("xlink:href").unwrap_or_default();
            let text = inline(&element.children, styles);
            format!("[{text}]({})", href.replace(' ', "%20"))
        }
        "text:line-break" => "  \n".to_string(),
        "text:tab" => " ".to_string(),
        "text:s" => " ".repeat(spaces(element)),
        _ if SKIPPED.contains(&element.name.as_str()) => String::new(),
        _ => inline(&element.children, styles),
    }
}

/// ``text:s text:c="3"`` is three spaces; a bare ``text:s`` is one.
fn spaces(element: &Element) -> usize {
    element.attr("text:c").and_then(|value| value.parse::<usize>().ok()).unwrap_or(1).clamp(1, 64)
}

/// The formatting the span's own style asks for. A span with no style is plain text, exactly as the
/// upstream reads it: ODF nests spans but never relies on inheritance for bold or italic.
fn style_of(element: &Element, styles: &Styles) -> TextStyle {
    styles.text.get(element.attr("text:style-name").unwrap_or_default()).copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(body: &str, styles: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <office:document-content>\
             <office:automatic-styles>{styles}</office:automatic-styles>\
             <office:body><office:text>{body}</office:text></office:body>\
             </office:document-content>"
        )
    }

    const BOLD: &str = "<style:style style:name=\"T1\" style:family=\"text\"><style:text-properties fo:font-weight=\"bold\"/></style:style>";
    const ITALIC: &str = "<style:style style:name=\"T2\" style:family=\"text\"><style:text-properties fo:font-style=\"italic\"/></style:style>";

    fn convert(body: &str, styles: &str) -> String {
        to_markdown(&doc(body, styles), "").expect("the document converts")
    }

    #[test]
    fn headings_keep_their_outline_level() {
        assert_eq!(convert("<text:h text:outline-level=\"2\">Conduta</text:h>", ""), "## Conduta\n");
        assert_eq!(convert("<text:h text:outline-level=\"9\">Fundo</text:h>", ""), "###### Fundo\n", "levels stop at six");
    }

    #[test]
    fn paragraphs_become_blocks() {
        assert_eq!(convert("<text:p>Um texto</text:p><text:p>Outro</text:p>", ""), "Um texto\n\nOutro\n");
    }

    #[test]
    fn empty_paragraphs_are_dropped() {
        assert_eq!(convert("<text:p/><text:p>x</text:p>", ""), "x\n");
    }

    #[test]
    fn bold_and_italic_come_from_the_style_definitions() {
        let text = convert("<text:p>a <text:span text:style-name=\"T1\">forte</text:span></text:p>", BOLD);
        assert_eq!(text, "a **forte**\n");
        let text = convert(
            "<text:p><text:span text:style-name=\"T1\"><text:span text:style-name=\"T2\">x</text:span></text:span></text:p>",
            &format!("{BOLD}{ITALIC}"),
        );
        assert_eq!(text, "***x***\n", "bold and italic nest");
    }

    #[test]
    fn an_unstyled_span_is_plain_text() {
        assert_eq!(convert("<text:p><text:span text:style-name=\"T9\">x</text:span></text:p>", ""), "x\n");
    }

    #[test]
    fn a_link_keeps_its_target() {
        assert_eq!(convert("<text:p><text:a xlink:href=\"https://x.com\">site</text:a></text:p>", ""), "[site](https://x.com)\n");
    }

    #[test]
    fn bullet_lists_use_dashes() {
        assert_eq!(
            convert(
                "<text:list><text:list-item><text:p>um</text:p></text:list-item><text:list-item><text:p>dois</text:p></text:list-item></text:list>",
                ""
            ),
            "- um\n- dois\n",
            "items of one list are one block"
        );
        assert_eq!(
            convert("<text:p>antes</text:p><text:list><text:list-item><text:p>um</text:p></text:list-item></text:list><text:p>depois</text:p>", ""),
            "antes\n\n- um\n\ndepois\n"
        );
    }

    #[test]
    fn a_numbered_list_style_counts_from_one() {
        let body = "<text:list text:style-name=\"L1\"><text:list-item><text:p>um</text:p></text:list-item><text:list-item><text:p>dois</text:p></text:list-item></text:list>";
        let styles = "<text:list-style style:name=\"L1\"><text:list-level-style-number text:level=\"1\"/></text:list-style>";
        assert_eq!(convert(body, styles), "1. um\n2. dois\n");
    }

    #[test]
    fn nested_lists_are_indented() {
        let body = "<text:list><text:list-item><text:p>pai</text:p>\
                    <text:list><text:list-item><text:p>filho</text:p></text:list-item></text:list>\
                    </text:list-item></text:list>";
        assert_eq!(convert(body, ""), "- pai\n  - filho\n");
    }

    #[test]
    fn the_first_row_is_the_header_of_a_table() {
        let body = "<table:table>\
            <table:table-row><table:table-cell><text:p>Droga</text:p></table:table-cell><table:table-cell><text:p>Dose</text:p></table:table-cell></table:table-row>\
            <table:table-row><table:table-cell><text:p>Nora</text:p></table:table-cell><table:table-cell><text:p>0,1</text:p></table:table-cell></table:table-row>\
            </table:table>";
        assert_eq!(convert(body, ""), "| Droga | Dose |\n| --- | --- |\n| Nora | 0,1 |\n");
    }

    #[test]
    fn an_explicit_header_block_wins() {
        let body = "<table:table>\
            <table:table-header-rows><table:table-row><table:table-cell><text:p>H</text:p></table:table-cell></table:table-row></table:table-header-rows>\
            <table:table-row><table:table-cell><text:p>c</text:p></table:table-cell></table:table-row>\
            </table:table>";
        assert_eq!(convert(body, ""), "| H |\n| --- |\n| c |\n");
    }

    #[test]
    fn a_repeated_cell_is_written_once_per_column() {
        let body = "<table:table><table:table-row>\
            <table:table-cell table:number-columns-repeated=\"3\"><text:p>x</text:p></table:table-cell>\
            </table:table-row></table:table>";
        assert_eq!(convert(body, ""), "| x | x | x |\n| --- | --- | --- |\n");
    }

    #[test]
    fn a_line_break_stays_a_line_break() {
        assert_eq!(convert("<text:p>a<text:line-break/>b</text:p>", ""), "a  \nb\n");
    }

    #[test]
    fn a_run_of_spaces_is_kept() {
        assert_eq!(convert("<text:p>a<text:s text:c=\"3\"/>b</text:p>", ""), "a   b\n");
    }

    #[test]
    fn text_that_looks_like_markup_is_escaped() {
        assert_eq!(convert("<text:p>1 &lt; 2 &amp; 3</text:p>", ""), "1 < 2 & 3\n", "a lone < is plain text");
        assert_eq!(convert("<text:p># não é título</text:p>", ""), "\\# não é título\n");
        assert_eq!(convert("<text:p>1. não é lista</text:p>", ""), "1\\. não é lista\n");
        assert_eq!(convert("<text:p>*não é ênfase*</text:p>", ""), "\\*não é ênfase\\*\n");
    }

    #[test]
    fn pictures_are_dropped_and_their_caption_kept() {
        let body = "<text:p><draw:frame><draw:image xlink:href=\"x.png\"/></draw:frame>legenda</text:p>";
        assert_eq!(convert(body, ""), "legenda\n");
    }

    #[test]
    fn a_document_without_a_body_is_empty() {
        assert_eq!(to_markdown("<office:document-content></office:document-content>", "").expect("valid"), "");
    }

    #[test]
    fn the_notes_body_is_found_through_the_office_wrapper() {
        // The real `content.xml` nests the text in `office:body`, with the styles beside it.
        let body = doc("<text:p>Olá</text:p>", "");
        assert_eq!(to_markdown(&body, "").expect("valid"), "Olá\n");
    }
}
