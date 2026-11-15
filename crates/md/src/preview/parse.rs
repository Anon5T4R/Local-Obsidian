//! pulldown-cmark events → the block model.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use super::{Align, Block, Span, Task};

/// What is open at this point of the parse. Inline containers are always on top of block ones.
enum Open {
    Paragraph(Vec<Span>),
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Quote(Vec<Block>),
    Item(Vec<Span>),
    List {
        ordered: bool,
        items: Vec<Vec<Span>>,
        tasks: Vec<Option<Task>>,
    },
    Emphasis(Vec<Span>),
    Strong(Vec<Span>),
    Strike(Vec<Span>),
    Link {
        spans: Vec<Span>,
        destination: String,
    },
    Image {
        alt: Vec<Span>,
        destination: String,
    },
    CodeBlock {
        language: String,
        text: String,
    },
    /// A table cell: inline content collected until the row moves on.
    Cell(Vec<Span>),
    Table {
        aligns: Vec<Align>,
        headers: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
        row: Vec<Vec<Span>>,
        in_head: bool,
    },
}

impl Open {
    fn holds_inline(&self) -> bool {
        !matches!(self, Open::Quote(_) | Open::List { .. } | Open::CodeBlock { .. } | Open::Table { .. })
    }
}

/// Parses `markdown` into blocks (callouts are [`super::callout`]'s job, one pass later).
pub(super) fn parse(markdown: &str, options: Options) -> Vec<Block> {
    let mut builder = Builder::default();
    for event in Parser::new_ext(markdown, options) {
        builder.event(event);
    }
    builder.finish()
}

#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    stack: Vec<Open>,
    /// The checkbox of the item being read, waiting for its `Item` to close.
    pending_task: Option<Task>,
    /// How many checkboxes the note has declared so far, in document order.
    task_count: usize,
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
            Event::Rule => self.push_block(Block::Rule),
            // A checkbox is drawn as a widget, not as `[x] ` text; the item it belongs to picks it
            // up when it closes.
            Event::TaskListMarker(done) => {
                self.pending_task = Some(Task { done, index: self.task_count });
                self.task_count += 1;
            }
            Event::FootnoteReference(name) => self.text(&format!("[^{name}]")),
            Event::InlineMath(tex) | Event::DisplayMath(tex) => self.span(Span::Code(tex.to_string())),
            // Raw HTML has no renderer here; showing it beats dropping it silently.
            Event::Html(html) | Event::InlineHtml(html) => self.text(&html),
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        let open = match tag {
            Tag::Paragraph | Tag::FootnoteDefinition(_) | Tag::HtmlBlock | Tag::MetadataBlock(_) => Open::Paragraph(Vec::new()),
            Tag::Heading { level, .. } => Open::Heading { level: level_number(level), spans: Vec::new() },
            Tag::BlockQuote(_) => Open::Quote(Vec::new()),
            Tag::CodeBlock(kind) => Open::CodeBlock { language: language_of(&kind), text: String::new() },
            Tag::List(start) => Open::List { ordered: start.is_some(), items: Vec::new(), tasks: Vec::new() },
            Tag::Item => Open::Item(Vec::new()),
            Tag::Emphasis => Open::Emphasis(Vec::new()),
            Tag::Strong => Open::Strong(Vec::new()),
            Tag::Strikethrough => Open::Strike(Vec::new()),
            Tag::Link { dest_url, .. } => Open::Link { spans: Vec::new(), destination: dest_url.to_string() },
            Tag::Image { dest_url, .. } => Open::Image { alt: Vec::new(), destination: dest_url.to_string() },
            Tag::Table(aligns) => Open::Table {
                aligns: aligns.iter().map(|a| align_of(*a)).collect(),
                headers: Vec::new(),
                rows: Vec::new(),
                row: Vec::new(),
                in_head: false,
            },
            Tag::TableHead => {
                if let Some(Open::Table { in_head, .. }) = self.stack.last_mut() {
                    *in_head = true;
                }
                return;
            }
            Tag::TableRow => {
                if let Some(Open::Table { row, .. }) = self.stack.last_mut() {
                    row.clear();
                }
                return;
            }
            Tag::TableCell => Open::Cell(Vec::new()),
            // Definition lists are flattened: their text shows, their structure does not.
            Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => return,
            _ => return,
        };
        self.stack.push(open);
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::FootnoteDefinition | TagEnd::HtmlBlock | TagEnd::MetadataBlock(_) => {
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
                if let Some(Open::List { ordered, items, tasks }) = self.stack.pop() {
                    self.push_block(Block::List { ordered, items, tasks });
                }
            }
            TagEnd::Item => {
                if let Some(Open::Item(spans)) = self.stack.pop() {
                    let task = self.pending_task.take();
                    match self.stack.last_mut() {
                        Some(Open::List { items, tasks, .. }) => {
                            items.push(spans);
                            tasks.push(task);
                        }
                        _ => self.push_block(Block::Paragraph(spans)),
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
            TagEnd::Link => {
                if let Some(Open::Link { spans, destination }) = self.stack.pop() {
                    self.span(Span::Link { spans, destination });
                }
            }
            TagEnd::Image => {
                if let Some(Open::Image { alt, destination }) = self.stack.pop() {
                    self.push_block(Block::Image { destination, alt });
                }
            }
            TagEnd::TableCell => {
                if let Some(Open::Cell(spans)) = self.stack.pop() {
                    for open in self.stack.iter_mut().rev() {
                        if let Open::Table { row, .. } = open {
                            row.push(spans);
                            break;
                        }
                    }
                }
            }
            TagEnd::TableRow => {
                if let Some(Open::Table { row, rows, in_head, headers, .. }) = self.stack.last_mut() {
                    let taken = std::mem::take(row);
                    if *in_head {
                        *headers = taken;
                    } else {
                        rows.push(taken);
                    }
                }
            }
            TagEnd::TableHead => {
                if let Some(Open::Table { in_head, headers, row, .. }) = self.stack.last_mut() {
                    // pulldown-cmark ends the head row at `TableHead`, not at a `TableRow`.
                    if !row.is_empty() {
                        *headers = std::mem::take(row);
                    }
                    *in_head = false;
                }
            }
            TagEnd::Table => {
                if let Some(Open::Table { aligns, headers, rows, .. }) = self.stack.pop() {
                    self.push_block(Block::Table { headers, rows, aligns });
                }
            }
            TagEnd::DefinitionList | TagEnd::DefinitionListTitle | TagEnd::DefinitionListDefinition => {}
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
            | Open::Cell(spans)
            | Open::Image { alt: spans, .. }
            | Open::Link { spans, .. } => spans,
            Open::Heading { spans, .. } => spans,
            _ => return,
        };
        match (spans.last_mut(), &span) {
            (Some(Span::Text(previous)), Span::Text(next)) => previous.push_str(next),
            _ => spans.push(span),
        }
    }

    /// Blocks go to the nearest block container above the block level (a quote), else to the root.
    fn push_block(&mut self, block: Block) {
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
                Open::List { ordered, items, tasks } => self.push_block(Block::List { ordered, items, tasks }),
                Open::CodeBlock { language, text } => self.push_block(Block::Code { language, text: text.trim_end_matches('\n').to_string() }),
                Open::Table { aligns, headers, rows, .. } => self.push_block(Block::Table { headers, rows, aligns }),
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

fn align_of(align: Alignment) -> Align {
    match align {
        Alignment::Center => Align::Center,
        Alignment::Right => Align::Right,
        // `None` and `Left` read the same here: the cell starts at its left edge.
        Alignment::Left | Alignment::None => Align::Left,
    }
}
