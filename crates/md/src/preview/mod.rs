//! The note preview: raw markdown in, the small block model the UI draws out.
//!
//! Two passes. [`prepare`] rewrites the syntax pulldown-cmark does not know into this app's own
//! dialect of plain Markdown (wikilinks become links to a `#wikilink:` destination, author-only
//! `%%comments%%` disappear), and [`blocks`] parses the result into [`Block`]s and promotes
//! `> [!kind]` quotes into [`Block::Callout`]s.
//!
//! The model is deliberately small - a reading view, not a typesetter. What pulldown knows and this
//! model does not (footnotes, raw HTML) still shows its text instead of vanishing.

mod callout;
mod parse;

use pulldown_cmark::Options;

use crate::code;
use crate::links;

/// One block of the reading view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph(Vec<Span>),
    List {
        ordered: bool,
        items: Vec<Vec<Span>>,
        /// The task state of each item, in the same order as `items`. Empty for a plain list; an
        /// item that is not a task is `None` in a checklist that mixes both.
        tasks: Vec<Option<Task>>,
    },
    Code {
        language: String,
        text: String,
    },
    Quote(Vec<Block>),
    /// `> [!warning] Título`: a callout, with the type the author wrote and its title.
    Callout {
        kind: String,
        title: String,
        body: Vec<Block>,
    },
    /// A table, cells inline like a paragraph.
    Table {
        headers: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
        aligns: Vec<Align>,
    },
    /// `![[imagem.png]]` / `![alt](_attachments/imagem.png)`.
    Image {
        destination: String,
        alt: Vec<Span>,
    },
    Rule,
}

/// A `- [ ]` / `- [x]` item in a checklist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Task {
    pub done: bool,
    /// Which task of the note this is, counting from zero: the reading view says "this one", and
    /// the editor finds it by the same count, so no offsets have to survive [`prepare`].
    pub index: usize,
}

/// A block's horizontal alignment, from the table's separator row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// A run of text inside a block, with the emphasis a note can carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    Text(String),
    Code(String),
    Strong(Vec<Span>),
    Emphasis(Vec<Span>),
    Strike(Vec<Span>),
    /// A `#tag` written in the note: the reading view draws it as a tag, not as prose.
    Tag(String),
    /// `==text==`: the author's highlight, drawn with a tinted background.
    Highlight(String),
    /// `destination` is a URL or this app's `#wikilink:<target>` form (see [`links::from_url`]).
    Link {
        spans: Vec<Span>,
        destination: String,
    },
}

impl Span {
    /// The text a span shows, emphasis stripped (tests, tooltips).
    pub fn plain(&self) -> String {
        match self {
            Span::Text(text) | Span::Code(text) => text.clone(),
            Span::Tag(tag) => format!("#{tag}"),
            Span::Highlight(text) => text.clone(),
            Span::Strong(spans) | Span::Emphasis(spans) | Span::Strike(spans) => spans.iter().map(Span::plain).collect(),
            Span::Link { spans, .. } => spans.iter().map(Span::plain).collect(),
        }
    }
}

impl Block {
    /// The block's own text, flattened (tests, search previews).
    pub fn plain(&self) -> String {
        let join = |spans: &[Span]| spans.iter().map(Span::plain).collect::<String>();
        match self {
            Block::Heading { spans, .. } | Block::Paragraph(spans) => join(spans),
            Block::List { items, .. } => items.iter().map(|item| join(item)).collect::<Vec<_>>().join("\n"),
            Block::Code { text, .. } => text.clone(),
            Block::Quote(inner) | Block::Callout { body: inner, .. } => inner.iter().map(Block::plain).collect::<Vec<_>>().join("\n"),
            Block::Table { headers, rows, .. } => {
                let mut lines: Vec<String> = vec![headers.iter().map(|cell| join(cell)).collect::<Vec<_>>().join(" | ")];
                lines.extend(rows.iter().map(|row| row.iter().map(|cell| join(cell)).collect::<Vec<_>>().join(" | ")));
                lines.join("\n")
            }
            Block::Image { alt, destination } => {
                if alt.is_empty() {
                    destination.clone()
                } else {
                    join(alt)
                }
            }
            Block::Rule => String::new(),
        }
    }
}

/// The note's markdown as it reaches [`blocks`]: frontmatter and comments gone, wikilinks turned
/// into links, and a callout header separated from the line under it.
pub fn prepare(markdown: &str) -> String {
    // The `---` block is metadata: the reading view must not show it as body text.
    let body = crate::frontmatter::body(markdown);
    let without_comments = strip_comments(body);
    let with_breaks = code::map_outside_code(&without_comments, separate_callout_headers);
    code::map_outside_code(&with_breaks, wikilinks_to_links)
}

/// `> [!note]` followed by the note's body on the next line is one paragraph to CommonMark, so the
/// header would swallow the first body line as its title. A hard break after the header keeps them
/// apart in the parsed text, and the callout pass splits on it.
fn separate_callout_headers(chunk: &str) -> String {
    let lines: Vec<&str> = chunk.split('\n').collect();
    let mut out = String::with_capacity(chunk.len() + 8);
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let content = line.trim_start();
        let content = content.trim_start_matches('>').trim_start();
        let is_header = content.starts_with("[!") && content.contains(']');
        // A hard break only means something when another line continues the paragraph.
        let continues = lines
            .get(index + 1)
            .map(|next| {
                let next = next.trim_start();
                !next.trim_start_matches('>').trim_start().is_empty() && !next.starts_with('#')
            })
            .unwrap_or(false);
        if is_header && continues && !line.ends_with('\\') {
            out.push_str(line);
            out.push('\\');
        } else {
            out.push_str(line);
        }
    }
    out
}

/// `%%text%%` is a note to the author: it stays in the file and never reaches the reading view.
pub fn strip_comments(markdown: &str) -> String {
    code::map_outside_code(markdown, |chunk| {
        let mut out = String::with_capacity(chunk.len());
        let mut rest = chunk;
        while let Some(open) = rest.find("%%") {
            out.push_str(&rest[..open]);
            match rest[open + 2..].find("%%") {
                Some(close) => rest = &rest[open + 2 + close + 2..],
                None => {
                    // An unclosed comment is not a comment.
                    out.push_str(&rest[open..]);
                    return out;
                }
            }
        }
        out.push_str(rest);
        out
    })
}

/// `[[Note]]` / `[[Note#Section|text]]` → `[text](#wikilink:Note%23Section)`.
fn wikilinks_to_links(chunk: &str) -> String {
    let mut out = String::with_capacity(chunk.len());
    let mut rest = chunk;
    while let Some(open) = rest.find("[[") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("]]") else {
            out.push_str(&rest[open..]);
            return out;
        };
        let inner = &after[..close];
        // `[[target|alias]]`: the target has no `]` and no `|`, the alias has no `]`.
        let (target, alias) = match inner.find('|') {
            Some(at) => (&inner[..at], Some(&inner[at + 1..])),
            None => (inner, None),
        };
        if target.contains(']') || alias.is_some_and(|a| a.contains(']') || a.is_empty()) {
            out.push_str("[[");
            out.push_str(inner);
            out.push_str("]]");
            rest = &after[close + 2..];
            continue;
        }
        let display = alias.map(str::to_string).unwrap_or_else(|| links::display_target(target));
        out.push('[');
        out.push_str(&display);
        out.push_str("](");
        out.push_str(&links::url(target));
        out.push(')');
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    out
}

/// Parses prepared markdown into blocks.
pub fn blocks(markdown: &str) -> Vec<Block> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_MATH);

    let parsed = parse::parse(markdown, options);
    callout::promote(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wikilink_becomes_a_link_to_the_vault_scheme() {
        let blocks = blocks(&prepare("veja [[Nota|a nota]]"));
        let Block::Paragraph(spans) = &blocks[0] else { panic!("{blocks:?}") };
        let Span::Link { spans, destination } = &spans[1] else { panic!("{spans:?}") };
        assert_eq!(spans.iter().map(Span::plain).collect::<String>(), "a nota");
        assert_eq!(links::from_url(destination).as_deref(), Some("Nota"));
    }

    #[test]
    fn a_wikilink_without_an_alias_shows_the_note_and_the_section() {
        let blocks = blocks(&prepare("[[Nota#Seção]]"));
        assert_eq!(blocks[0].plain(), "Nota › Seção");
    }

    #[test]
    fn a_wikilink_inside_code_stays_literal() {
        let prepared = prepare("`[[Nota]]`");
        assert_eq!(prepared, "`[[Nota]]`");
        assert_eq!(blocks(&prepared)[0].plain(), "[[Nota]]");
    }

    #[test]
    fn comments_are_dropped_outside_code_only() {
        assert_eq!(strip_comments("a %%segredo%% b"), "a  b");
        assert_eq!(strip_comments("`%%x%%`"), "`%%x%%`");
        assert_eq!(strip_comments("a %%unclosed"), "a %%unclosed");
    }

    #[test]
    fn headings_lists_quotes_and_fences_survive() {
        let blocks = blocks("# Título\n\n- um\n- dois\n\n> citado\n\n```rust\nfn x() {}\n```\n\n---\n");
        assert!(matches!(blocks[0], Block::Heading { level: 1, .. }), "{blocks:?}");
        assert!(matches!(blocks[1], Block::List { ordered: false, .. }), "{blocks:?}");
        assert!(matches!(blocks[2], Block::Quote(_)), "{blocks:?}");
        assert!(matches!(&blocks[3], Block::Code { language, .. } if language == "rust"), "{blocks:?}");
        assert!(matches!(blocks[4], Block::Rule), "{blocks:?}");
    }

    #[test]
    fn bold_and_italic_nest_inside_a_paragraph() {
        let blocks = blocks("um **forte** e *ênfase*");
        let Block::Paragraph(spans) = &blocks[0] else { panic!("{blocks:?}") };
        assert!(matches!(&spans[1], Span::Strong(_)), "{spans:?}");
        assert!(matches!(&spans[3], Span::Emphasis(_)), "{spans:?}");
    }

    #[test]
    fn a_nested_list_comes_after_its_parent() {
        // The nested list is a block of its own (an item holds spans only) and has to land *after*
        // the list that contains it - before this it came out inside out, child list first.
        let blocks = blocks("- pai\n  - filho\n- irmão");
        let lists: Vec<&Block> = blocks.iter().filter(|block| matches!(block, Block::List { .. })).collect();
        assert_eq!(lists.len(), 2, "{blocks:?}");
        let Block::List { items, .. } = lists[0] else { panic!() };
        assert_eq!(items.len(), 2, "the parent keeps its items: {items:?}");
        let Block::List { items, .. } = lists[1] else { panic!() };
        assert_eq!(items.len(), 1, "the nested one holds the child: {items:?}");
        assert!(lists[0].plain().contains("pai") && lists[0].plain().contains("irmão"), "{lists:?}");
        assert!(lists[1].plain().contains("filho"));
    }

    #[test]
    fn a_highlight_is_its_own_run() {
        let blocks = blocks("um ==destaque== e outro");
        let Block::Paragraph(spans) = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(spans[0], Span::Text("um ".into()));
        assert_eq!(spans[1], Span::Highlight("destaque".into()));
        assert_eq!(spans[2], Span::Text(" e outro".into()));
        assert_eq!(blocks[0].plain(), "um destaque e outro");
    }

    #[test]
    fn an_odd_pair_of_equals_stays_text() {
        assert_eq!(blocks("a == b")[0].plain().trim(), "a == b");
        assert_eq!(blocks("== aberto")[0].plain().trim(), "== aberto");
    }

    #[test]
    fn a_checkbox_keeps_its_state_and_its_place_in_the_note() {
        let blocks = blocks("- [x] feito\n- [ ] pendente");
        let Block::List { items, tasks, .. } = &blocks[0] else { panic!("{blocks:?}") };
        // The marker is a widget, not text: the item's text starts after the box.
        assert_eq!(items[0].iter().map(Span::plain).collect::<String>(), "feito");
        assert_eq!(items[1].iter().map(Span::plain).collect::<String>(), "pendente");
        assert_eq!(tasks[0], Some(Task { done: true, index: 0 }));
        assert_eq!(tasks[1], Some(Task { done: false, index: 1 }));
    }

    #[test]
    fn a_plain_list_has_no_tasks() {
        let blocks = blocks("- um\n- dois");
        let Block::List { tasks, .. } = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(tasks, &vec![None, None]);
    }

    #[test]
    fn the_task_indexes_count_across_lists() {
        let blocks = blocks("- [ ] um\n\n1. [x] dois");
        let mut indexes = Vec::new();
        for block in &blocks {
            if let Block::List { tasks, .. } = block {
                indexes.extend(tasks.iter().flatten().map(|task| (task.index, task.done)));
            }
        }
        assert_eq!(indexes, [(0, false), (1, true)]);
    }

    #[test]
    fn adjacent_text_is_merged() {
        assert_eq!(blocks("a\\*b")[0].plain(), "a*b");
    }

    #[test]
    fn a_table_keeps_its_cells_and_its_alignment() {
        let blocks = blocks("| a | b | c |\n|:--|:-:|--:|\n| 1 | 2 | 3 |");
        let Block::Table { headers, rows, aligns } = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(headers.len(), 3);
        assert_eq!(headers[0].iter().map(Span::plain).collect::<String>(), "a");
        assert_eq!(rows[0][2].iter().map(Span::plain).collect::<String>(), "3");
        assert_eq!(aligns, &vec![Align::Left, Align::Center, Align::Right]);
    }

    #[test]
    fn the_frontmatter_never_reaches_the_reading_view() {
        let blocks = blocks(&prepare("---\ntags: [cardio]\naliases: [SCA]\n---\n# Sepse\n\ntexto"));
        assert!(!blocks[0].plain().contains("cardio"), "{blocks:?}");
        assert!(matches!(&blocks[0], Block::Heading { level: 1, .. }), "{blocks:?}");
        assert_eq!(blocks.len(), 2, "{blocks:?}");
    }

    #[test]
    fn a_table_cell_keeps_its_inline_formatting() {
        let blocks = blocks("| a |\n|---|\n| **forte** |");
        let Block::Table { rows, .. } = &blocks[0] else { panic!("{blocks:?}") };
        assert!(matches!(&rows[0][0][0], Span::Strong(_)), "{:?}", rows[0]);
    }

    #[test]
    fn an_image_is_a_block_with_its_alt_text() {
        let blocks = blocks("![um gato](_attachments/gato.png)");
        let Block::Image { destination, alt } = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(destination, "_attachments/gato.png");
        assert_eq!(alt.iter().map(Span::plain).collect::<String>(), "um gato");
    }

    #[test]
    fn a_wikilink_image_keeps_its_target() {
        // `![[gato.png]]`: what the embed pass did not expand stays an image link.
        let blocks = blocks(&prepare("![[gato.png]]"));
        assert!(blocks.iter().any(|block| matches!(block, Block::Image { .. })), "{blocks:?}");
    }

    #[test]
    fn a_callout_over_a_quote_is_a_callout() {
        let blocks = blocks(&prepare("> [!warning] Cuidado\n>\n> não faça isso"));
        let Block::Callout { kind, title, body } = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!((kind.as_str(), title.as_str()), ("warning", "Cuidado"));
        assert_eq!(body.len(), 1);
    }
}
