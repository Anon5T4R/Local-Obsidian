//! The note preview: raw markdown in, the small block model the UI draws out.
//!
//! Two passes. [`prepare`] rewrites the syntax pulldown-cmark does not know into this app's own
//! dialect of plain Markdown (wikilinks become links to a `#wikilink:` destination, author-only
//! `%%comments%%` disappear), and [`blocks`] parses the result into [`Block`]s.
//!
//! The model is deliberately small - a reading view, not a typesetter. Anything pulldown knows and
//! this model does not (tables, footnotes, raw HTML) still shows its text instead of vanishing.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::code;
use crate::links;

/// One block of the reading view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading { level: u8, spans: Vec<Span> },
    Paragraph(Vec<Span>),
    List { ordered: bool, items: Vec<Vec<Span>> },
    Code { language: String, text: String },
    Quote(Vec<Block>),
    Rule,
}

/// A run of text inside a block, with the emphasis a note can carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    Text(String),
    Code(String),
    Strong(Vec<Span>),
    Emphasis(Vec<Span>),
    Strike(Vec<Span>),
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
            Span::Strong(spans) | Span::Emphasis(spans) | Span::Strike(spans) => spans.iter().map(Span::plain).collect(),
            Span::Link { spans, .. } => spans.iter().map(Span::plain).collect(),
        }
    }
}

/// The note's markdown as it reaches [`blocks`]: comments gone, wikilinks turned into links.
pub fn prepare(markdown: &str) -> String {
    let without_comments = strip_comments(markdown);
    code::map_outside_code(&without_comments, wikilinks_to_links)
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

    let mut builder = Builder::default();
    for event in Parser::new_ext(markdown, options) {
        builder.event(event);
    }
    builder.finish()
}

/// What is open at this point of the parse. Inline containers are always on top of block ones.
enum Open {
    Paragraph(Vec<Span>),
    Heading { level: u8, spans: Vec<Span> },
    Quote(Vec<Block>),
    Item(Vec<Span>),
    List { ordered: bool, items: Vec<Vec<Span>> },
    Emphasis(Vec<Span>),
    Strong(Vec<Span>),
    Strike(Vec<Span>),
    Link { spans: Vec<Span>, destination: String },
    CodeBlock { language: String, text: String },
}

impl Open {
    fn holds_inline(&self) -> bool {
        !matches!(self, Open::Quote(_) | Open::List { .. } | Open::CodeBlock { .. })
    }
}

#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    stack: Vec<Open>,
}

impl Builder {
    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(text) => self.span(Span::Code(text.to_string())),
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.text("\n"),
            Event::Rule => self.blocks.push(Block::Rule),
            Event::TaskListMarker(done) => self.text(if done { "[x] " } else { "[ ] " }),
            Event::FootnoteReference(name) => self.text(&format!("[^{name}]")),
            Event::InlineMath(tex) | Event::DisplayMath(tex) => self.span(Span::Code(tex.to_string())),
            // Raw HTML has no renderer here; showing it beats dropping it silently.
            Event::Html(html) | Event::InlineHtml(html) => self.text(&html),
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        let open = match tag {
            Tag::Paragraph | Tag::TableCell | Tag::FootnoteDefinition(_) | Tag::HtmlBlock | Tag::MetadataBlock(_) => Open::Paragraph(Vec::new()),
            Tag::Heading { level, .. } => Open::Heading { level: level_number(level), spans: Vec::new() },
            Tag::BlockQuote(_) => Open::Quote(Vec::new()),
            Tag::CodeBlock(kind) => Open::CodeBlock { language: language_of(&kind), text: String::new() },
            Tag::List(start) => Open::List { ordered: start.is_some(), items: Vec::new() },
            Tag::Item => Open::Item(Vec::new()),
            Tag::Emphasis => Open::Emphasis(Vec::new()),
            Tag::Strong => Open::Strong(Vec::new()),
            Tag::Strikethrough => Open::Strike(Vec::new()),
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => Open::Link { spans: Vec::new(), destination: dest_url.to_string() },
            // Tables and definition lists: their structure is flattened into paragraphs per cell,
            // so the words still show in reading order.
            Tag::Table(_) | Tag::TableHead | Tag::TableRow | Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => return,
            _ => return,
        };
        self.stack.push(open);
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::TableCell | TagEnd::FootnoteDefinition | TagEnd::HtmlBlock | TagEnd::MetadataBlock(_) => {
                if let Some(Open::Paragraph(spans)) = self.stack.pop() {
                    self.push_block(Block::Paragraph(spans));
                }
            }
            TagEnd::Heading(_) => {
                if let Some(Open::Heading { level, spans }) = self.stack.pop() {
                    self.push_block(Block::Heading { level, spans });
                }
            }
            TagEnd::BlockQuote(_) => {
                if let Some(Open::Quote(blocks)) = self.stack.pop() {
                    self.push_block(Block::Quote(blocks));
                }
            }
            TagEnd::CodeBlock => {
                if let Some(Open::CodeBlock { language, text }) = self.stack.pop() {
                    self.push_block(Block::Code { language, text: text.trim_end_matches('\n').to_string() });
                }
            }
            TagEnd::List(_) => {
                if let Some(Open::List { ordered, items }) = self.stack.pop() {
                    self.push_block(Block::List { ordered, items });
                }
            }
            TagEnd::Item => {
                if let Some(Open::Item(spans)) = self.stack.pop() {
                    if let Some(Open::List { items, .. }) = self.stack.last_mut() {
                        items.push(spans);
                    } else {
                        self.push_block(Block::Paragraph(spans));
                    }
                }
            }
            TagEnd::Emphasis => {
                if let Some(Open::Emphasis(spans)) = self.stack.pop() {
                    self.span(Span::Emphasis(spans));
                }
            }
            TagEnd::Strong => {
                if let Some(Open::Strong(spans)) = self.stack.pop() {
                    self.span(Span::Strong(spans));
                }
            }
            TagEnd::Strikethrough => {
                if let Some(Open::Strike(spans)) = self.stack.pop() {
                    self.span(Span::Strike(spans));
                }
            }
            TagEnd::Link | TagEnd::Image => {
                if let Some(Open::Link { spans, destination }) = self.stack.pop() {
                    self.span(Span::Link { spans, destination });
                }
            }
            TagEnd::Table
            | TagEnd::TableHead
            | TagEnd::TableRow
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {}
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        if let Some(Open::CodeBlock { text: buffer, .. }) = self.stack.last_mut() {
            buffer.push_str(text);
            return;
        }
        self.span(Span::Text(text.to_string()));
    }

    fn span(&mut self, span: Span) {
        if self.stack.last().is_none_or(|open| !open.holds_inline()) {
            self.stack.push(Open::Paragraph(Vec::new()));
        }
        let Some(open) = self.stack.last_mut() else { return };
        let spans = match open {
            Open::Paragraph(spans)
            | Open::Item(spans)
            | Open::Emphasis(spans)
            | Open::Strong(spans)
            | Open::Strike(spans)
            | Open::Link { spans, .. } => spans,
            Open::Heading { spans, .. } => spans,
            _ => return,
        };
        match (spans.last_mut(), &span) {
            (Some(Span::Text(previous)), Span::Text(next)) => previous.push_str(next),
            _ => spans.push(span),
        }
    }

    fn push_block(&mut self, block: Block) {
        // The nearest block container above the block level holds it; the root otherwise.
        for open in self.stack.iter_mut().rev() {
            if let Open::Quote(blocks) = open {
                blocks.push(block);
                return;
            }
        }
        self.blocks.push(block);
    }

    fn finish(mut self) -> Vec<Block> {
        while let Some(open) = self.stack.pop() {
            match open {
                Open::Paragraph(spans) => self.push_block(Block::Paragraph(spans)),
                Open::Heading { level, spans } => self.push_block(Block::Heading { level, spans }),
                Open::Quote(blocks) => self.push_block(Block::Quote(blocks)),
                Open::List { ordered, items } => self.push_block(Block::List { ordered, items }),
                Open::CodeBlock { language, text } => self.push_block(Block::Code { language, text: text.trim_end_matches('\n').to_string() }),
                _ => {}
            }
        }
        self.blocks
    }
}

fn level_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn language_of(kind: &CodeBlockKind<'_>) -> String {
    match kind {
        CodeBlockKind::Fenced(info) => info.trim().to_string(),
        CodeBlockKind::Indented => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraph_text(blocks: &[Block]) -> String {
        match blocks {
            [Block::Paragraph(spans), ..] => spans.iter().map(Span::plain).collect(),
            _ => panic!("expected a paragraph, got {blocks:?}"),
        }
    }

    #[test]
    fn a_wikilink_becomes_a_link_to_the_vault_scheme() {
        let prepared = prepare("veja [[Nota|a nota]]");
        let blocks = blocks(&prepared);
        let Block::Paragraph(spans) = &blocks[0] else { panic!("{blocks:?}") };
        let Span::Link { spans, destination } = &spans[1] else { panic!("{spans:?}") };
        assert_eq!(spans.iter().map(Span::plain).collect::<String>(), "a nota");
        assert_eq!(links::from_url(destination).as_deref(), Some("Nota"));
    }

    #[test]
    fn a_wikilink_without_an_alias_shows_the_note_and_the_section() {
        let blocks = blocks(&prepare("[[Nota#Seção]]"));
        let Block::Paragraph(spans) = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(spans.iter().map(Span::plain).collect::<String>(), "Nota › Seção");
    }

    #[test]
    fn a_wikilink_inside_code_stays_literal() {
        let prepared = prepare("`[[Nota]]`");
        assert_eq!(prepared, "`[[Nota]]`");
        assert_eq!(paragraph_text(&blocks(&prepared)), "[[Nota]]");
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
    fn a_checkbox_shows_its_state_as_text() {
        let blocks = blocks("- [x] feito\n- [ ] pendente");
        let Block::List { items, .. } = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(items[0].iter().map(Span::plain).collect::<String>(), "[x] feito");
        assert_eq!(items[1].iter().map(Span::plain).collect::<String>(), "[ ] pendente");
    }

    #[test]
    fn a_table_keeps_its_words() {
        let blocks = blocks("| a | b |\n|---|---|\n| 1 | 2 |");
        let text: String = blocks
            .iter()
            .map(|block| match block {
                Block::Paragraph(spans) => spans.iter().map(Span::plain).collect::<String>(),
                other => format!("{other:?}"),
            })
            .collect();
        for word in ["a", "b", "1", "2"] {
            assert!(text.contains(word), "{text}");
        }
    }

    #[test]
    fn adjacent_text_is_merged() {
        let blocks = blocks("a\\*b");
        assert_eq!(paragraph_text(&blocks), "a*b");
    }
}
