//! A note as one static HTML page.
//!
//! The reading view already turns a note into [`Block`]s; this writes the same blocks as HTML with the
//! stylesheet inlined, so the file opens anywhere with no server, no JavaScript and no assets beyond
//! the pictures the note itself shows.
//!
//! Three decisions worth knowing:
//!
//! - **Colours follow the reader's system**, not the app's theme: the page carries a light palette and
//!   a `prefers-color-scheme: dark` block, so a file shared with someone else looks right on their
//!   machine (and the palette is this app's violet).
//! - **A wikilink becomes `<Note>.html`**, relative and in the same folder. Exporting the notes it
//!   points at into that folder is what makes the link work; nothing is invented for a note that was
//!   never exported.
//! - **A picture keeps its vault path** (`_attachments/foto.png`), so copying the folder beside the
//!   page is enough for it to show - [`images`] lists what has to be copied.

use crate::links;
use crate::outline::slug;
use crate::preview::{Align, Block, Span};

/// The whole page: the note's title, the reader's language and the stylesheet, in one file.
pub fn document(title: &str, language: &str, blocks: &[Block]) -> String {
    let language = if language.is_empty() { "en" } else { language };
    format!(
        "<!DOCTYPE html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{}</title>\n<style>\n{STYLE}\n</style>\n</head>\n<body>\n<main>\n{}\n</main>\n</body>\n</html>\n",
        escape(language),
        escape(title),
        blocks_html(blocks)
    )
}

/// The note's own markup, without the page around it.
pub fn body(blocks: &[Block]) -> String {
    blocks_html(blocks)
}

/// Where the page's pictures live, as the note wrote them: what an export has to copy.
pub fn images(blocks: &[Block]) -> Vec<String> {
    let mut out = Vec::new();
    collect_images(blocks, &mut out);
    out
}

fn collect_images(blocks: &[Block], out: &mut Vec<String>) {
    for block in blocks {
        match block {
            Block::Image { destination, .. } => {
                let source = image_source(destination);
                if !source.is_empty() && !out.contains(&source) {
                    out.push(source);
                }
            }
            Block::Quote(inner) | Block::Callout { body: inner, .. } => collect_images(inner, out),
            _ => {}
        }
    }
}

/// The blocks as HTML, one per line so the file reads well if someone opens it in an editor.
fn blocks_html(blocks: &[Block]) -> String {
    blocks.iter().map(block_html).filter(|html| !html.is_empty()).collect::<Vec<_>>().join("\n")
}

fn block_html(block: &Block) -> String {
    match block {
        Block::Heading { level, spans } => {
            let level = (*level).clamp(1, 6);
            let text = spans_html(spans);
            // The id is what a `[[Nota#Seção]]` from another exported note lands on.
            let id = slug(&plain(spans));
            if id.is_empty() { format!("<h{level}>{text}</h{level}>") } else { format!("<h{level} id=\"{}\">{text}</h{level}>", escape(&id)) }
        }
        Block::Paragraph(spans) => {
            // An empty paragraph is nothing to show (a note with only a picture gets one).
            if plain(spans).trim().is_empty() { String::new() } else { format!("<p>{}</p>", spans_html(spans)) }
        }
        Block::List { ordered, items, tasks } => {
            let (open, close) = if *ordered { ("ol", "ol") } else { ("ul", "ul") };
            let mut out = format!("<{open}>");
            for (index, item) in items.iter().enumerate() {
                let task = tasks.get(index).copied().flatten();
                match task {
                    Some(task) => {
                        let checked = if task.done { " checked" } else { "" };
                        let class = if task.done { "task done" } else { "task" };
                        out.push_str(&format!("<li class=\"{class}\"><input type=\"checkbox\" disabled{checked}> {}</li>", spans_html(item)));
                    }
                    None => out.push_str(&format!("<li>{}</li>", spans_html(item))),
                }
            }
            out.push_str(&format!("</{close}>"));
            out
        }
        Block::Code { language, text } => {
            let class =
                if language.trim().is_empty() { String::new() } else { format!(" class=\"language-{}\"", escape(&language.trim().to_lowercase())) };
            format!("<pre><code{class}>{}</code></pre>", escape(text))
        }
        Block::Quote(inner) => format!("<blockquote>\n{}\n</blockquote>", blocks_html(inner)),
        Block::Callout { kind, title, body } => {
            let class = kind_class(kind);
            let mut out = format!("<div class=\"callout callout-{class}\">");
            if !title.trim().is_empty() {
                out.push_str(&format!("\n<div class=\"callout-title\">{}</div>", escape(title)));
            }
            let inner = blocks_html(body);
            if !inner.is_empty() {
                out.push_str(&format!("\n{inner}"));
            }
            out.push_str("\n</div>");
            out
        }
        Block::Table { headers, rows, aligns } => {
            let mut out = String::from("<table>\n<thead>\n");
            out.push_str(&row_html(headers, aligns, "th"));
            out.push_str("\n</thead>\n<tbody>");
            for row in rows {
                out.push('\n');
                out.push_str(&row_html(row, aligns, "td"));
            }
            out.push_str("\n</tbody>\n</table>");
            out
        }
        Block::Image { destination, alt } => {
            let source = image_source(destination);
            if source.is_empty() {
                return String::new();
            }
            let alt = alt.iter().map(Span::plain).collect::<String>();
            format!("<img src=\"{}\" alt=\"{}\">", escape(&href_encode(&source)), escape(&alt))
        }
        Block::Rule => "<hr>".to_string(),
    }
}

fn row_html(cells: &[Vec<Span>], aligns: &[Align], tag: &str) -> String {
    let mut out = String::from("<tr>");
    for (index, cell) in cells.iter().enumerate() {
        let style = match aligns.get(index).copied().unwrap_or_default() {
            Align::Left => "",
            Align::Center => " style=\"text-align:center\"",
            Align::Right => " style=\"text-align:right\"",
        };
        out.push_str(&format!("<{tag}{style}>{}</{tag}>", spans_html(cell)));
    }
    out.push_str("</tr>");
    out
}

fn spans_html(spans: &[Span]) -> String {
    let mut out = String::new();
    for span in spans {
        match span {
            Span::Text(text) => out.push_str(&escape(text)),
            Span::Tag(tag) => out.push_str(&format!("<span class=\"tag\">#{}</span>", escape(tag))),
            Span::Highlight(text) => out.push_str(&format!("<mark>{}</mark>", escape(text))),
            Span::Code(text) => out.push_str(&format!("<code>{}</code>", escape(text))),
            Span::Strong(inner) => out.push_str(&format!("<strong>{}</strong>", spans_html(inner))),
            Span::Emphasis(inner) => out.push_str(&format!("<em>{}</em>", spans_html(inner))),
            Span::Strike(inner) => out.push_str(&format!("<del>{}</del>", spans_html(inner))),
            Span::Link { spans, destination } => out.push_str(&link_html(spans, destination)),
        }
    }
    out
}

/// A link: a wikilink points at the note's exported page, a web link opens in a new tab, and
/// anything else is written as it came.
fn link_html(spans: &[Span], destination: &str) -> String {
    let text = spans_html(spans);
    match links::from_url(destination) {
        Some(target) => {
            let (note, section) = match target.split_once('#') {
                Some((note, section)) => (note.trim(), section.trim()),
                None => (target.trim(), ""),
            };
            let file = note.rsplit('/').next().unwrap_or(note).trim_end_matches(".md");
            let mut href = format!("{}.html", href_encode(file));
            let anchor = slug(section);
            if !anchor.is_empty() {
                href.push('#');
                href.push_str(&anchor);
            }
            format!("<a href=\"{}\">{text}</a>", escape(&href))
        }
        None if destination.starts_with("http://") || destination.starts_with("https://") => {
            format!("<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{text}</a>", escape(destination))
        }
        None => format!("<a href=\"{}\">{text}</a>", escape(&href_encode(destination))),
    }
}

/// The picture a block points at, with the `#wikilink:` scheme removed.
fn image_source(destination: &str) -> String {
    links::from_url(destination).unwrap_or_else(|| destination.to_string()).trim().to_string()
}

/// The callout class: the note's own type when it is a word, `note` otherwise.
fn kind_class(kind: &str) -> String {
    let cleaned: String = kind.trim().to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == '-').collect();
    if cleaned.is_empty() { "note".to_string() } else { cleaned }
}

/// The plain text of a run of spans (heading ids, alt text).
fn plain(spans: &[Span]) -> String {
    spans.iter().map(Span::plain).collect()
}

/// HTML-escapes text. A note may hold `<`, `&` and quotes as content, and none of it is markup.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Percent-encodes what a browser would otherwise cut out of a relative path: spaces, `#`, `?`, `%`.
fn href_encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            other => out.push(other),
        }
    }
    out
}

/// The stylesheet, inlined: one palette for a light screen, one for a dark one, and the callout
/// colours the note's own types ask for.
const STYLE: &str = r#":root {
  color-scheme: light dark;
  --bg: #fdfcff; --fg: #20232b; --muted: #5f6673; --line: #e4e3ec; --panel: #f5f4fa;
  --link: #5b52d6; --accent: #746af0;
}
@media (prefers-color-scheme: dark) {
  :root { --bg: #1b1c20; --fg: #e6e7ec; --muted: #a3a8b3; --line: #33363d; --panel: #24262c;
          --link: #b3adff; --accent: #8b83ff; }
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--fg);
       font: 16px/1.65 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Noto Sans", Arial, sans-serif; }
main { max-width: 46rem; margin: 0 auto; padding: 2.5rem 1.25rem 4rem; }
h1, h2, h3, h4, h5, h6 { line-height: 1.25; margin: 2rem 0 .7rem; }
h1 { font-size: 1.9rem; } h2 { font-size: 1.5rem; } h3 { font-size: 1.25rem; } h4 { font-size: 1.08rem; }
h1:first-child { margin-top: 0; }
p { margin: .9em 0; }
a { color: var(--link); text-decoration: none; }
a:hover { text-decoration: underline; }
ul, ol { padding-left: 1.4rem; }
li { margin: .25rem 0; }
li.task { list-style: none; }
li.task.done { color: var(--muted); }
li.task input { margin-right: .5rem; vertical-align: middle; }
code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace;
       font-size: .9em; background: var(--panel); padding: .1em .35em; border-radius: 4px; }
pre { background: var(--panel); padding: 1rem; border-radius: 8px; overflow-x: auto; }
pre code { background: none; padding: 0; font-size: .88rem; }
blockquote { margin: 1.2em 0; padding: .1em 0 .1em 1rem; border-left: 3px solid var(--line); color: var(--muted); }
hr { border: 0; border-top: 1px solid var(--line); margin: 2rem 0; }
table { border-collapse: collapse; width: 100%; margin: 1.2em 0; font-size: .95rem; }
th, td { border: 1px solid var(--line); padding: .4rem .65rem; text-align: left; vertical-align: top; }
thead th { background: var(--panel); }
img { max-width: 100%; height: auto; border-radius: 6px; }
.callout { margin: 1.2em 0; padding: .7rem 1rem; border-left: 3px solid var(--c, #3b82f6);
           background: var(--cb, rgba(59, 130, 246, .08)); border-radius: 0 8px 8px 0; }
.callout-title { font-weight: 600; color: var(--c, #3b82f6); margin-bottom: .3em; }
.callout > :first-child { margin-top: 0; }
.callout > :last-child { margin-bottom: 0; }
.callout-note, .callout-info { --c: #3b82f6; --cb: rgba(59, 130, 246, .08); }
.callout-tip, .callout-success, .callout-done { --c: #16a34a; --cb: rgba(22, 163, 74, .09); }
.callout-warning, .callout-caution, .callout-attention { --c: #d97706; --cb: rgba(217, 119, 6, .1); }
.callout-danger, .callout-error, .callout-bug, .callout-failure { --c: #dc2626; --cb: rgba(220, 38, 38, .09); }
.callout-question, .callout-help, .callout-faq { --c: #9333ea; --cb: rgba(147, 51, 234, .09); }
.callout-example, .callout-quote, .callout-abstract, .callout-summary, .callout-tldr { --c: #0d9488; --cb: rgba(13, 148, 136, .09); }
.callout-embed { --c: var(--accent); --cb: rgba(116, 106, 240, .1); }
.callout-card, .callout-flashcard, .callout-mnemonic, .callout-pergunta { --c: #db2777; --cb: rgba(219, 39, 119, .09); }
.tag { background: rgba(116, 106, 240, .14); color: var(--accent); border-radius: 10px; padding: .05em .5em; font-size: .92em; white-space: nowrap; }
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::{self, Task};

    fn page(markdown: &str) -> String {
        body(&preview::blocks(&preview::prepare(markdown)))
    }

    #[test]
    fn a_heading_keeps_its_level_and_gets_an_id() {
        assert_eq!(page("## Sepse"), "<h2 id=\"sepse\">Sepse</h2>");
        assert_eq!(page("# Acentuação é vida"), "<h1 id=\"acentuação-é-vida\">Acentuação é vida</h1>");
    }

    #[test]
    fn a_paragraph_keeps_its_emphasis() {
        assert_eq!(page("um **forte**, *ênfase* e `código`"), "<p>um <strong>forte</strong>, <em>ênfase</em> e <code>código</code></p>");
    }

    #[test]
    fn text_that_looks_like_markup_is_escaped() {
        assert_eq!(page("1 < 2 & \"aspas\""), "<p>1 &lt; 2 &amp; &quot;aspas&quot;</p>");
    }

    #[test]
    fn a_wikilink_points_at_the_exported_page() {
        assert_eq!(page("ver [[Sepse]]"), "<p>ver <a href=\"Sepse.html\">Sepse</a></p>");
    }

    #[test]
    fn a_wikilink_to_a_section_lands_on_the_heading() {
        // The preview shows the section in the link text; the href carries it as the anchor.
        assert_eq!(page("ver [[Sepse#Tríade do qSOFA]]"), "<p>ver <a href=\"Sepse.html#tríade-do-qsofa\">Sepse › Tríade do qSOFA</a></p>");
    }

    #[test]
    fn a_note_name_with_spaces_is_encoded_in_the_href() {
        assert_eq!(page("[[Sepse neonatal]]"), "<p><a href=\"Sepse%20neonatal.html\">Sepse neonatal</a></p>");
    }

    #[test]
    fn a_web_link_opens_in_a_new_tab() {
        assert_eq!(page("[site](https://x.com)"), "<p><a href=\"https://x.com\" target=\"_blank\" rel=\"noopener noreferrer\">site</a></p>");
    }

    #[test]
    fn a_checklist_keeps_its_boxes_without_being_editable() {
        let html = page("- [x] feito\n- [ ] pendente");
        assert!(html.contains("<li class=\"task done\"><input type=\"checkbox\" disabled checked> feito</li>"), "{html}");
        assert!(html.contains("<li class=\"task\"><input type=\"checkbox\" disabled> pendente</li>"), "{html}");
    }

    #[test]
    fn a_numbered_list_is_an_ol() {
        let html = page("1. um\n2. dois");
        assert!(html.starts_with("<ol><li>um</li><li>dois</li></ol>"), "{html}");
    }

    #[test]
    fn a_code_block_keeps_its_language() {
        assert_eq!(page("```rust\nfn x() {}\n```"), "<pre><code class=\"language-rust\">fn x() {}</code></pre>");
        assert_eq!(page("```\nplain\n```"), "<pre><code>plain</code></pre>");
    }

    #[test]
    fn a_callout_becomes_a_coloured_box() {
        let html = page("> [!warning] Cuidado\n> não misture");
        assert!(html.starts_with("<div class=\"callout callout-warning\">"), "{html}");
        assert!(html.contains("<div class=\"callout-title\">Cuidado</div>"), "{html}");
        assert!(html.contains("<p>não misture</p>"), "{html}");
        assert!(html.ends_with("</div>"), "{html}");
    }

    #[test]
    fn a_card_is_its_own_colour() {
        let html = page("> [!card]- Pergunta?\n> resposta");
        assert!(html.contains("callout-card"), "{html}");
    }

    #[test]
    fn a_table_keeps_its_header_and_alignment() {
        let html = page("| a | b |\n|:--|--:|\n| 1 | 2 |");
        assert!(html.contains("<thead>"), "{html}");
        assert!(html.contains("<th>a</th>"), "{html}");
        assert!(html.contains("<th style=\"text-align:right\">b</th>"), "{html}");
        assert!(html.contains("<td>1</td>"), "{html}");
    }

    #[test]
    fn a_picture_keeps_its_vault_path() {
        // The alt text is the file's own name, and the empty paragraph a lone picture leaves behind
        // is not written.
        let html = page("![[foto um.png]]");
        assert_eq!(html, "<img src=\"foto%20um.png\" alt=\"foto um.png\">");
        assert_eq!(images(&preview::blocks(&preview::prepare("![[foto um.png]]"))), ["foto um.png"]);
    }

    #[test]
    fn the_pictures_of_a_callout_are_listed_too() {
        let blocks = preview::blocks(&preview::prepare("> [!note] Foto\n> ![[a.png]]"));
        assert_eq!(images(&blocks), ["a.png"]);
    }

    #[test]
    fn a_rule_is_an_hr() {
        assert_eq!(page("---"), "<hr>");
    }

    #[test]
    fn the_page_wraps_the_note_with_the_stylesheet() {
        let html = document("Sepse & choque", "pt-BR", &[]);
        assert!(html.starts_with("<!DOCTYPE html>"), "{html}");
        assert!(html.contains("<html lang=\"pt-BR\">"), "{html}");
        assert!(html.contains("<title>Sepse &amp; choque</title>"), "{html}");
        assert!(html.contains("prefers-color-scheme: dark"), "the palette follows the reader's system");
        assert!(html.contains("<main>"), "{html}");
    }

    #[test]
    fn a_document_without_a_language_falls_back_to_english() {
        assert!(document("Nota", "", &[]).contains("<html lang=\"en\">"));
    }

    #[test]
    fn the_page_is_well_formed() {
        // A page that opens a tag and never closes it still "looks" finished in a browser, so the
        // counts are what keep the renderer honest.
        let rich = "# Sepse\n\nTexto com **forte** e [[Outra]].\n\n- [ ] um\n- [x] dois\n\n1. a\n2. b\n\n\
                    > [!warning] Cuidado\n> não misture\n\n> citação\n\n| a | b |\n|:--|--:|\n| 1 | 2 |\n\n\
                    ```rust\nfn x() {}\n```\n\n---\n\n![[foto.png]]\n";
        let html = page(rich);
        let opens = |tag: &str| html.matches(&format!("<{tag}>")).count() + html.matches(&format!("<{tag} ")).count();
        for tag in [
            "p",
            "ul",
            "ol",
            "li",
            "table",
            "tr",
            "th",
            "td",
            "div",
            "blockquote",
            "pre",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "strong",
            "em",
            "code",
            "a",
            "img",
        ] {
            let closes = html.matches(&format!("</{tag}>")).count();
            if tag == "img" {
                continue; // void element: no closing tag
            }
            assert_eq!(opens(tag), closes, "<{tag}> unbalanced in\n{html}");
        }
        assert!(html.contains("<hr>"), "{html}");
        assert!(html.contains("<img"), "{html}");
    }

    #[test]
    fn a_task_that_came_from_the_model_is_drawn_once() {
        // The model carries the box separately from the text; the export must not repeat it.
        let blocks =
            vec![Block::List { ordered: false, items: vec![vec![Span::Text("feito".into())]], tasks: vec![Some(Task { done: true, index: 0 })] }];
        assert_eq!(body(&blocks), "<ul><li class=\"task done\"><input type=\"checkbox\" disabled checked> feito</li></ul>");
    }
}
