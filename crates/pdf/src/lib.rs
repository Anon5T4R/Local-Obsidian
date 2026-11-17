//! A note as a PDF: the reading view's blocks laid out on A4 pages and written as **real text**.
//!
//! The text is glyph runs with embedded (subsetted) fonts and a Unicode mapping taken from the
//! original string, so a PDF reader can select, search and copy what it shows. What it draws, in
//! this phase: headings, paragraphs with the inline styles (bold, italic, strike, code, highlight,
//! tags, links), lists and checklists, quotes, callouts, tables with a grid, code blocks, rules and
//! a page number. What it does not draw yet: pictures (they print as `[image: …]`) and link
//! annotations - the next phase.
//!
//! The crate owns layout and drawing, not policy: the app brings the faces (its own regular and
//! mono, plus the craft-fonts fallbacks) and the blocks. The page is what print has used for a
//! century: A4, 56 pt margins (about 20 mm), a 483 pt measure.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::path::Path;
use std::sync::Arc;

use craft_pdf::write::{self, Metadata, Writer, backend as krilla};
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect as PdfRect, Size, Transform};
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use localobsidian_md::preview::{Align, Block, Span};
use skrifa::instance::{LocationRef, Size as SkSize};
use skrifa::{FontRef, MetadataProvider};

/// A4 in points: the paper a note is printed on.
pub const PAGE: (f32, f32) = (595.276, 841.89);
/// The margin on every side: about 20 mm, the measure print has used for a century.
pub const MARGIN: f32 = 56.0;
const CONTENT: f32 = PAGE.0 - 2.0 * MARGIN;
const BODY: f32 = 10.5;
const LINE: f32 = 15.6;

/// Text on paper.
const INK: Rgb = Rgb(0.13, 0.13, 0.15);
const WEAK: Rgb = Rgb(0.42, 0.42, 0.46);
/// The violet of the icon (the light theme's accent): tags and links on paper.
const ACCENT: Rgb = Rgb(0.42, 0.36, 0.79);

/// A colour, each channel in 0..1 (sRGB).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb(pub f32, pub f32, pub f32);

/// One font file the app brings.
#[derive(Debug, Clone)]
pub struct Face {
    pub data: Arc<Vec<u8>>,
    /// The face index inside a collection (`.ttc`); 0 for a plain `.ttf`/`.otf`.
    pub index: u32,
}

impl Face {
    pub fn new(data: impl Into<Vec<u8>>) -> Self {
        Self { data: Arc::new(data.into()), index: 0 }
    }
}

/// The faces a document is written with. `fallbacks` are tried in order for a character the regular
/// face does not have (the craft-fonts faces: CJK, Arabic, Hebrew, Devanagari).
pub struct Faces {
    pub regular: Face,
    pub mono: Face,
    pub fallbacks: Vec<Face>,
}

/// What the export did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub pages: usize,
    pub bytes: usize,
}

/// Why an export failed. The caller shows the reason; nothing is written on failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Write(String),
    Pdf(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Write(reason) | Self::Pdf(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for Error {}

/// Lays `blocks` out on A4 pages and writes them to `path` as a PDF titled `title`.
///
/// The file is written atomically: a failure leaves no half document behind.
pub fn export(blocks: &[Block], title: &str, faces: &Faces, path: &Path) -> Result<Report, Error> {
    let loaded = Loaded::new(faces)?;
    let mut sheet = Sheet::new(title.to_string());
    sheet.blocks(blocks, MARGIN, CONTENT, &loaded);
    let pages = sheet.pages.len();
    let bytes = sheet.write(&loaded)?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| Error::Write(format!("{}: {error}", folder.display())))?;
    }
    write_atomic(path, &bytes).map_err(|error| Error::Write(format!("{}: {error}", path.display())))?;
    Ok(Report { pages, bytes: bytes.len() })
}

/// A font file ready to draw with: the backend's font (for the page) and the parsed metrics (for
/// the layout).
struct LoadedFace {
    data: Arc<Vec<u8>>,
    index: u32,
    pdf: Font,
}

impl LoadedFace {
    fn new(face: &Face) -> Result<Self, Error> {
        let pdf = write::font(&face.data, face.index, None).map_err(|error| Error::Pdf(format!("{error}")))?;
        Ok(Self { data: face.data.clone(), index: face.index, pdf })
    }

    /// The parsed font, for glyph ids, advances and vertical metrics.
    fn parsed(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(&self.data, self.index).ok()
    }

    fn glyph(&self, character: char) -> Option<u32> {
        self.parsed()?.charmap().map(character).map(|glyph| glyph.to_u32())
    }

    /// The advance of `character` as a fraction of the em: what the PDF backend takes (it scales by
    /// the font size). Half an em when the face has no glyph, so a character without a face still
    /// takes room instead of stacking on the one before.
    fn advance_em(&self, character: char) -> f32 {
        let Some(font) = self.parsed() else { return 0.5 };
        let Some(glyph) = font.charmap().map(character) else { return 0.5 };
        let metrics = font.metrics(SkSize::unscaled(), LocationRef::default());
        if metrics.units_per_em == 0 {
            return 0.5;
        }
        let units = font.glyph_metrics(SkSize::unscaled(), LocationRef::default()).advance_width(glyph).unwrap_or(0.0);
        if units <= 0.0 {
            return 0.5;
        }
        units / f32::from(metrics.units_per_em)
    }

    /// The advance of `character` at `size`, in points: the layout walks in points.
    fn advance(&self, character: char, size: f32) -> f32 {
        self.advance_em(character) * size
    }
}

/// Which of the loaded faces draws a character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Regular,
    Mono,
    Fallback(usize),
}

/// The faces in use, by role.
struct Loaded {
    regular: LoadedFace,
    mono: LoadedFace,
    fallbacks: Vec<LoadedFace>,
}

impl Loaded {
    fn new(faces: &Faces) -> Result<Self, Error> {
        Ok(Self {
            regular: LoadedFace::new(&faces.regular)?,
            mono: LoadedFace::new(&faces.mono)?,
            fallbacks: faces.fallbacks.iter().map(LoadedFace::new).collect::<Result<Vec<_>, _>>()?,
        })
    }

    fn face(&self, slot: Slot) -> &LoadedFace {
        match slot {
            Slot::Regular => &self.regular,
            Slot::Mono => &self.mono,
            // The index comes from `slot`, never from outside.
            Slot::Fallback(index) => self.fallbacks.get(index).unwrap_or(&self.regular),
        }
    }

    /// The face that draws `character`: mono keeps its own face, everything else prefers the
    /// regular one and falls back in order (the craft-fonts faces).
    fn slot(&self, character: char, mono: bool) -> Slot {
        if mono && self.mono.glyph(character).is_some() {
            return Slot::Mono;
        }
        if self.regular.glyph(character).is_some() {
            return Slot::Regular;
        }
        for (index, fallback) in self.fallbacks.iter().enumerate() {
            if fallback.glyph(character).is_some() {
                return Slot::Fallback(index);
            }
        }
        Slot::Regular
    }
}

/// One piece of inline text, already resolved to a look.
#[derive(Debug, Clone)]
struct Piece {
    text: String,
    size: f32,
    mono: bool,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    highlight: bool,
    accent: bool,
    underline: bool,
    color: Rgb,
}

impl Piece {
    fn plain(text: impl Into<String>, size: f32) -> Self {
        Self {
            text: text.into(),
            size,
            mono: false,
            bold: false,
            italic: false,
            strike: false,
            code: false,
            highlight: false,
            accent: false,
            underline: false,
            color: INK,
        }
    }
}

/// What the page draws, in order.
enum Op {
    /// A run of glyphs. `text` is the original string, which is what the reader extracts; `glyphs`
    /// carry ids, advances and the range of `text` each one stands for.
    Glyphs {
        at: Point,
        slot: Slot,
        glyphs: Vec<KrillaGlyph>,
        text: String,
        size: f32,
        color: Rgb,
        weight: f32,
        shear: f32,
    },
    Rect {
        rect: PdfRect,
        color: Rgb,
    },
    Line {
        from: Point,
        to: Point,
        color: Rgb,
        width: f32,
    },
}

struct Page {
    ops: Vec<Op>,
}

/// The document being built: the pages, and a cursor that walks down the current one.
struct Sheet {
    title: String,
    pages: Vec<Page>,
    y: f32,
    /// The tinted band a quote's or a callout's lines are drawn over: set while its body is laid
    /// out, so every line of it carries the same background and the same left bar.
    band: Option<Rgb>,
}

impl Sheet {
    fn new(title: String) -> Self {
        Self { title, pages: vec![Page { ops: Vec::new() }], y: MARGIN, band: None }
    }

    fn page(&mut self) -> &mut Page {
        let last = self.pages.len().saturating_sub(1);
        &mut self.pages[last]
    }

    /// The top of the next line: a new page starts when `height` does not fit.
    fn room(&mut self, height: f32) -> f32 {
        if self.y + height > PAGE.1 - MARGIN {
            self.pages.push(Page { ops: Vec::new() });
            self.y = MARGIN;
        }
        let top = self.y;
        self.y += height;
        top
    }

    fn blocks(&mut self, blocks: &[Block], x: f32, width: f32, fonts: &Loaded) {
        for block in blocks {
            match block {
                Block::Heading { level, spans } => {
                    let size = match level {
                        1 => 21.0,
                        2 => 16.5,
                        3 => 13.5,
                        _ => 11.5,
                    };
                    self.y += if *level <= 2 { 10.0 } else { 7.0 };
                    let mut pieces = inline(spans);
                    for piece in &mut pieces {
                        piece.bold = true;
                        piece.size = size;
                    }
                    self.flow(&pieces, x, width, size * 1.35, fonts, None);
                    self.y += 4.0;
                }
                Block::Paragraph(spans) => {
                    let pieces = inline(spans);
                    self.flow(&pieces, x, width, LINE, fonts, None);
                    self.y += 5.0;
                }
                Block::List { ordered, items, tasks } => {
                    for (index, item) in items.iter().enumerate() {
                        let task = tasks.get(index).copied().flatten();
                        // A nested item keeps its indentation in the text the parser hands over.
                        let spaces = leading_spaces(item);
                        let level = (spaces / 2).min(6);
                        let inset = f32::from(level as u16) * 14.0;
                        let marker = match task {
                            Some(_) => String::new(),
                            None if *ordered => format!("{}.", index + 1),
                            None => "•".to_string(),
                        };
                        let top = self.room(LINE);
                        let baseline = top + LINE * 0.76;
                        if let Some(task) = task {
                            self.checkbox(x + inset + 1.0, top + 4.0, task.done);
                        } else {
                            let mut piece = Piece::plain(marker, BODY);
                            piece.color = WEAK;
                            self.run(&piece, x + inset, baseline, fonts);
                        }
                        let mut pieces = inline(item);
                        for piece in &mut pieces {
                            piece.size = BODY;
                            piece.text = piece.text.trim_start().to_string();
                        }
                        let lines = wrap(&pieces, width - 18.0 - inset, fonts);
                        for (line_index, line) in lines.iter().enumerate() {
                            let baseline = if line_index == 0 {
                                baseline
                            } else {
                                let top = self.room(LINE);
                                top + LINE * 0.76
                            };
                            self.line(line, x + 18.0 + inset, baseline, fonts);
                        }
                    }
                    self.y += 3.0;
                }
                Block::Quote(body) => {
                    let saved = self.band;
                    self.band = Some(Rgb(0.62, 0.62, 0.72));
                    self.blocks(body, x + 14.0, width - 14.0, fonts);
                    self.band = saved;
                    self.y += 2.0;
                }
                Block::Callout { kind, title, body } => {
                    let color = callout_color(kind);
                    self.y += 3.0;
                    let saved = self.band;
                    self.band = Some(color);
                    if !title.trim().is_empty() {
                        let mut piece = Piece::plain(title.trim(), 11.0);
                        piece.bold = true;
                        piece.color = color;
                        self.flow(&[piece], x + 14.0, width - 20.0, 15.5, fonts, Some((color, 0.10)));
                    }
                    self.blocks(body, x + 14.0, width - 20.0, fonts);
                    self.band = saved;
                    self.y += 8.0;
                }
                Block::Table { headers, rows, aligns } => {
                    self.table(headers, rows, aligns, x, width, fonts);
                    self.y += 8.0;
                }
                Block::Code { language, text } => self.code(language, text, x, width, fonts),
                Block::Query { source } => self.code("query", source, x, width, fonts),
                Block::Image { destination, alt } => {
                    let text = alt.iter().map(Span::plain).collect::<String>();
                    let label = if text.trim().is_empty() { format!("[image: {destination}]") } else { format!("[image: {text}]") };
                    let mut piece = Piece::plain(label, BODY);
                    piece.color = WEAK;
                    piece.italic = true;
                    self.flow(&[piece], x, width, LINE, fonts, None);
                    self.y += 5.0;
                }
                Block::Rule => {
                    let top = self.room(10.0);
                    let y = top + 4.0;
                    self.line_op(Point::from_xy(x, y), Point::from_xy(x + width, y), Rgb(0.80, 0.80, 0.84), 0.8);
                    self.y += 8.0;
                }
            }
        }
    }

    /// One paragraph: wraps the pieces and draws each line, with an optional tinted band behind.
    ///
    /// The band (a quote's or a callout's) is the sheet's current one, so a block nested in another
    /// keeps the outer tint and gets a left bar of the same colour.
    fn flow(&mut self, pieces: &[Piece], x: f32, width: f32, height: f32, fonts: &Loaded, band: Option<(Rgb, f32)>) {
        let band = band.or(self.band.map(|color| (color, 0.10)));
        for line in wrap(pieces, width, fonts) {
            let top = self.room(height);
            let baseline = top + height * 0.76;
            if let Some((color, amount)) = band {
                if let Some(rect) = PdfRect::from_xywh(x - 6.0, top, width + 12.0, height) {
                    self.page().ops.push(Op::Rect { rect, color: tint(color, amount) });
                }
                if let Some(bar) = PdfRect::from_xywh(x - 12.0, top, 2.5, height) {
                    self.page().ops.push(Op::Rect { rect: bar, color });
                }
            }
            self.line(&line, x, baseline, fonts);
        }
    }

    /// Draws one laid-out line at `baseline`.
    fn line(&mut self, line: &[Piece], x: f32, baseline: f32, fonts: &Loaded) {
        let mut at = x;
        for piece in line {
            let width = advance(piece, fonts);
            let size = piece.size;
            if piece.highlight || piece.code {
                let color = if piece.code { Rgb(0.94, 0.94, 0.96) } else { tint(ACCENT, 0.16) };
                if let Some(rect) = PdfRect::from_xywh(at - 1.0, baseline - size * 0.86, width + 2.0, size * 1.24) {
                    self.page().ops.push(Op::Rect { rect, color });
                }
            }
            self.run(piece, at, baseline, fonts);
            if piece.strike || piece.underline {
                let color = if piece.accent { ACCENT } else { piece.color };
                let y = if piece.strike { baseline - size * 0.30 } else { baseline + 1.7 };
                self.line_op(Point::from_xy(at, y), Point::from_xy(at + width, y), color, 0.7);
            }
            at += width;
        }
    }

    /// One run of glyphs, split by face so a CJK character inside a Latin word keeps its font.
    fn run(&mut self, piece: &Piece, x: f32, baseline: f32, fonts: &Loaded) {
        if piece.text.is_empty() {
            return;
        }
        let color = if piece.accent { ACCENT } else { piece.color };
        let weight = if piece.bold { 0.32 } else { 0.0 };
        let shear = if piece.italic { -0.21 } else { 0.0 };
        let mono = piece.mono || piece.code;
        let mut start = 0usize;
        let mut pen = x;
        let mut current = fonts.slot(piece.text.chars().next().unwrap_or(' '), mono);
        for (index, character) in piece.text.char_indices() {
            let slot = fonts.slot(character, mono);
            if slot == current {
                continue;
            }
            let chunk = &piece.text[start..index];
            let run = glyphs_for(chunk, start, current, piece.size, fonts);
            let advance: f32 = run.iter().map(|glyph| glyph.x_advance * piece.size).sum();
            self.page().ops.push(Op::Glyphs {
                at: Point::from_xy(pen, baseline),
                slot: current,
                glyphs: run,
                // The whole piece: the glyph ranges are absolute in it (what a reader extracts).
                text: piece.text.clone(),
                size: piece.size,
                color,
                weight,
                shear,
            });
            pen += advance;
            start = index;
            current = slot;
        }
        let chunk = &piece.text[start..];
        let run = glyphs_for(chunk, start, current, piece.size, fonts);
        self.page().ops.push(Op::Glyphs {
            at: Point::from_xy(pen, baseline),
            slot: current,
            glyphs: run,
            text: piece.text.clone(),
            size: piece.size,
            color,
            weight,
            shear,
        });
    }

    fn line_op(&mut self, from: Point, to: Point, color: Rgb, width: f32) {
        self.page().ops.push(Op::Line { from, to, color, width });
    }

    /// A task's box, ticked when done.
    fn checkbox(&mut self, x: f32, top: f32, done: bool) {
        let size = 8.5;
        if let Some(rect) = PdfRect::from_xywh(x, top, size, size) {
            self.page().ops.push(Op::Rect { rect, color: Rgb(0.80, 0.80, 0.85) });
        }
        if done && let Some(rect) = PdfRect::from_xywh(x + 1.8, top + 1.8, size - 3.6, size - 3.6) {
            self.page().ops.push(Op::Rect { rect, color: ACCENT });
        }
    }

    /// A code block: a tinted band per line, the language in the corner, the text in mono.
    fn code(&mut self, language: &str, text: &str, x: f32, width: f32, fonts: &Loaded) {
        let band = (Rgb(0.50, 0.50, 0.60), 0.06);
        self.y += 2.0;
        if !language.trim().is_empty() {
            let mut label = Piece::plain(language.trim(), 8.0);
            label.color = WEAK;
            label.mono = true;
            self.flow(&[label], x + 8.0, width - 16.0, 11.0, fonts, Some(band));
        }
        for source in text.lines() {
            let mut piece = Piece::plain(source, 9.0);
            piece.mono = true;
            self.flow(&[piece], x + 8.0, width - 16.0, 13.0, fonts, Some(band));
        }
        self.y += 5.0;
    }

    /// A table: a header band, alternating rows, a grid, columns sized to their content and fitted
    /// to the measure; a cell that does not fit its column wraps, and the row grows with it.
    fn table(&mut self, headers: &[Vec<Span>], rows: &[Vec<Vec<Span>>], aligns: &[Align], x: f32, width: f32, fonts: &Loaded) {
        let columns = headers.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
        if columns == 0 {
            return;
        }
        const PAD: f32 = 5.0;
        const CELL_LINE: f32 = 11.5;
        let size = 9.5;
        let cells = |row: &[Vec<Span>]| -> Vec<Vec<Piece>> {
            (0..columns).map(|index| row.get(index).map(|cell| inline_at(cell, size)).unwrap_or_default()).collect()
        };
        let header = cells(headers);
        let body: Vec<Vec<Vec<Piece>>> = rows.iter().map(|row| cells(row)).collect();

        // Columns as wide as their own content, then fitted to the measure (never below 20 pt; if
        // the floor pushes the table past it, everything shrinks once more).
        let mut widths = vec![24.0f32; columns];
        for (index, width_of) in widths.iter_mut().enumerate() {
            let mut natural = text_width(header.get(index).cloned().unwrap_or_default().as_slice(), fonts);
            for row in &body {
                natural = natural.max(text_width(row.get(index).cloned().unwrap_or_default().as_slice(), fonts));
            }
            *width_of = natural + 2.0 * PAD;
        }
        let total: f32 = widths.iter().sum();
        let mut factor = if total > width { width / total } else { 1.0 };
        let fitted: f32 = widths.iter().map(|width_of| (width_of * factor).max(20.0)).sum();
        if fitted > width {
            factor *= width / fitted;
        }
        if factor != 1.0 {
            for width_of in &mut widths {
                *width_of = (*width_of * factor).max(20.0);
            }
        }

        let wrapped = |cells: &[Vec<Piece>]| -> Vec<Vec<Vec<Piece>>> {
            widths
                .iter()
                .enumerate()
                .map(|(index, width_of)| wrap(cells.get(index).cloned().unwrap_or_default().as_slice(), (width_of - 2.0 * PAD).max(1.0), fonts))
                .collect()
        };
        let header_lines = wrapped(&header);
        let header_height = height_of(&header_lines, CELL_LINE);
        let top = self.room(header_height);
        if let Some(rect) = PdfRect::from_xywh(x, top, width, header_height) {
            self.page().ops.push(Op::Rect { rect, color: Rgb(0.94, 0.94, 0.96) });
        }
        self.cells_into(&header_lines, &widths, aligns, x, top, true, CELL_LINE, fonts);
        let mut y = top + header_height;
        self.hline(x, y, width, 0.8);

        for (index, row) in body.iter().enumerate() {
            let row_lines = wrapped(row);
            let height = height_of(&row_lines, CELL_LINE);
            if y + height > PAGE.1 - MARGIN {
                self.pages.push(Page { ops: Vec::new() });
                self.y = MARGIN;
                y = MARGIN;
            }
            if index % 2 == 1
                && let Some(rect) = PdfRect::from_xywh(x, y, width, height)
            {
                self.page().ops.push(Op::Rect { rect, color: Rgb(0.975, 0.975, 0.985) });
            }
            self.cells_into(&row_lines, &widths, aligns, x, y, false, CELL_LINE, fonts);
            y += height;
            self.hline(x, y, width, 0.5);
        }
        self.y = y;
        // The vertical separators, over the whole table.
        let mut cursor = x;
        for width_of in &widths {
            self.vline(cursor, top, y, 0.5);
            cursor += width_of;
        }
        self.vline(cursor, top, y, 0.5);
    }

    /// Draws one row's cells from its pre-wrapped lines, each line under the one before.
    #[allow(clippy::too_many_arguments)]
    fn cells_into(
        &mut self,
        lines: &[Vec<Vec<Piece>>],
        widths: &[f32],
        aligns: &[Align],
        x: f32,
        top: f32,
        bold: bool,
        cell_line: f32,
        fonts: &Loaded,
    ) {
        let empty: Vec<Vec<Piece>> = Vec::new();
        let mut cursor = x;
        for (index, width_of) in widths.iter().enumerate() {
            let inner = (width_of - 10.0).max(1.0);
            let cell_lines = lines.get(index).unwrap_or(&empty);
            for (line_index, line) in cell_lines.iter().enumerate() {
                let used = text_width(line, fonts);
                let offset = match aligns.get(index).copied().unwrap_or(Align::Left) {
                    Align::Center => (inner - used) / 2.0,
                    Align::Right => inner - used,
                    Align::Left => 0.0,
                };
                let baseline = top + cell_line * (line_index as f32 + 0.85);
                let mut pen = cursor + 5.0 + offset.max(0.0);
                for piece in line {
                    let mut piece = piece.clone();
                    piece.bold |= bold;
                    self.run(&piece, pen, baseline, fonts);
                    pen += advance(&piece, fonts);
                }
            }
            cursor += width_of;
        }
    }

    fn hline(&mut self, x: f32, y: f32, width: f32, thickness: f32) {
        let from = Point::from_xy(x, y);
        let to = Point::from_xy(x + width, y);
        self.line_op(from, to, Rgb(0.80, 0.80, 0.86), thickness);
    }

    fn vline(&mut self, x: f32, top: f32, bottom: f32, thickness: f32) {
        let from = Point::from_xy(x, top);
        let to = Point::from_xy(x, bottom);
        self.line_op(from, to, Rgb(0.84, 0.84, 0.88), thickness);
    }

    /// Serializes the pages, with a page number in the footer.
    fn write(self, fonts: &Loaded) -> Result<Vec<u8>, Error> {
        let settings = krilla::SerializeSettings { compress_content_streams: true, enable_tagging: false, ..Default::default() };
        let metadata =
            Metadata { creator: "Local Obsidian".to_string(), producer: "Local Obsidian".to_string(), title: Some(self.title), ..Default::default() };
        let mut writer = Writer::new(settings, metadata);
        let size = Size::from_wh(PAGE.0, PAGE.1).ok_or_else(|| Error::Pdf("the page size is invalid".to_string()))?;
        let total = self.pages.len();
        {
            let document = writer.document_mut();
            for (index, page) in self.pages.iter().enumerate() {
                let mut started = document.start_page_with(PageSettings::new(size));
                let mut surface = started.surface();
                for op in &page.ops {
                    draw(&mut surface, op, fonts);
                }
                let mut number = Piece::plain(format!("{} / {total}", index + 1), 8.0);
                number.color = WEAK;
                let baseline = PAGE.1 - MARGIN + 22.0;
                let width = advance(&number, fonts);
                let mut run = glyphs_for(&number.text, 0, Slot::Regular, number.size, fonts);
                let at = Point::from_xy((PAGE.0 - width) / 2.0, baseline);
                surface.set_stroke(None);
                surface.set_fill(Some(fill(WEAK, 1.0)));
                run.shrink_to_fit();
                surface.draw_glyphs(at, &run, fonts.regular.pdf.clone(), &number.text, number.size, false);
            }
        }
        writer.finish().map_err(|error| Error::Pdf(format!("{error}")))
    }
}

/// Draws one op on the page surface.
fn draw(surface: &mut Surface, op: &Op, fonts: &Loaded) {
    match op {
        Op::Rect { rect, color } => {
            let mut builder = PathBuilder::new();
            builder.push_rect(*rect);
            if let Some(path) = builder.finish() {
                surface.set_stroke(None);
                surface.set_fill(Some(fill(*color, 1.0)));
                surface.draw_path(&path);
            }
        }
        Op::Line { from, to, color, width } => {
            let mut builder = PathBuilder::new();
            builder.move_to(from.x, from.y);
            builder.line_to(to.x, to.y);
            if let Some(path) = builder.finish() {
                surface.set_fill(None);
                surface.set_stroke(Some(stroke(*color, *width)));
                surface.draw_path(&path);
            }
        }
        Op::Glyphs { at, slot, glyphs, text, size, color, weight, shear } => {
            let face = fonts.face(*slot);
            surface.set_fill(Some(fill(*color, 1.0)));
            if *weight > 0.0 {
                // Synthetic bold: the run is filled and its outline stroked. The backend draws the
                // text once (the stroke is a path), so the text stays selectable.
                surface.set_stroke(Some(stroke(*color, *size * 0.035)));
            } else {
                surface.set_stroke(None);
            }
            let sheared = *shear != 0.0;
            if sheared {
                surface.push_transform(&Transform::from_row(1.0, 0.0, *shear, 1.0, -*shear * at.y, 0.0));
            }
            surface.draw_glyphs(*at, glyphs, face.pdf.clone(), text, *size, false);
            if sheared {
                surface.pop();
            }
        }
    }
}

/// The glyphs of `text`, which starts at `start` in the piece, with their ranges in the piece.
///
/// The advances are em fractions: the backend scales them by the font size.
fn glyphs_for(text: &str, start: usize, slot: Slot, _size: f32, fonts: &Loaded) -> Vec<KrillaGlyph> {
    let face = fonts.face(slot);
    let mut glyphs = Vec::new();
    for (offset, character) in text.char_indices() {
        let advance = face.advance_em(character);
        let glyph_id = face.glyph(character).unwrap_or(0);
        glyphs.push(KrillaGlyph {
            glyph_id: GlyphId::new(glyph_id),
            text_range: start + offset..start + offset + character.len_utf8(),
            x_advance: advance,
            x_offset: 0.0,
            y_offset: 0.0,
            y_advance: 0.0,
            location: None,
        });
    }
    glyphs
}

/// The inline spans of a paragraph as pieces.
fn inline(spans: &[Span]) -> Vec<Piece> {
    inline_at(spans, BODY)
}

fn inline_at(spans: &[Span], size: f32) -> Vec<Piece> {
    let mut out = Vec::new();
    for span in spans {
        push_span(span, size, &Look::default(), &mut out);
    }
    out
}

/// The look a span inherits from the ones around it.
#[derive(Debug, Clone, Copy, Default)]
struct Look {
    bold: bool,
    italic: bool,
    strike: bool,
}

fn push_span(span: &Span, size: f32, look: &Look, out: &mut Vec<Piece>) {
    let mut base = Piece::plain(String::new(), size);
    base.bold = look.bold;
    base.italic = look.italic;
    base.strike = look.strike;
    match span {
        Span::Text(text) => out.push(Piece { text: text.clone(), ..base }),
        Span::Code(text) => out.push(Piece { text: text.clone(), code: true, color: Rgb(0.30, 0.18, 0.42), ..base }),
        Span::Highlight(text) => out.push(Piece { text: text.clone(), highlight: true, ..base }),
        Span::Tag(tag) => out.push(Piece { text: format!("#{tag}"), accent: true, ..base }),
        Span::Link { spans, .. } => {
            let mut pieces = Vec::new();
            for span in spans {
                push_span(span, size, look, &mut pieces);
            }
            for mut piece in pieces {
                piece.accent = true;
                piece.underline = true;
                out.push(piece);
            }
        }
        Span::Strong(spans) => {
            let inner = Look { bold: true, ..*look };
            for span in spans {
                push_span(span, size, &inner, out);
            }
        }
        Span::Emphasis(spans) => {
            let inner = Look { italic: true, ..*look };
            for span in spans {
                push_span(span, size, &inner, out);
            }
        }
        Span::Strike(spans) => {
            let inner = Look { strike: true, ..*look };
            for span in spans {
                push_span(span, size, &inner, out);
            }
        }
    }
}

/// The height of a row: the tallest cell's lines, plus the padding.
fn height_of(lines: &[Vec<Vec<Piece>>], cell_line: f32) -> f32 {
    let count = lines.iter().map(Vec::len).max().unwrap_or(1).max(1);
    count as f32 * cell_line + 5.0
}

/// How many spaces a list item starts with (its nesting, as the parser hands it over).
fn leading_spaces(spans: &[Span]) -> usize {
    for span in spans {
        match span {
            Span::Text(text) => {
                let spaces = text.chars().take_while(|character| *character == ' ').count();
                if spaces > 0 || !text.is_empty() {
                    return spaces;
                }
            }
            _ => return 0,
        }
    }
    0
}

/// Greedy wrapping: the spaces stay with the word before them, so a line never starts with one.
fn wrap(pieces: &[Piece], width: f32, fonts: &Loaded) -> Vec<Vec<Piece>> {
    let mut lines: Vec<Vec<Piece>> = vec![Vec::new()];
    let mut cursor = 0.0f32;
    for word in words(pieces) {
        let advance = self_advance(&word, fonts);
        let empty = lines.last().is_none_or(|line| line.is_empty());
        if cursor + advance > width && !empty && !word.text.trim().is_empty() {
            lines.push(Vec::new());
            cursor = 0.0;
        }
        if cursor == 0.0 && word.text.trim().is_empty() && lines.last().is_some_and(|line| line.is_empty()) {
            continue;
        }
        cursor += advance;
        if let Some(line) = lines.last_mut() {
            line.push(word);
        }
    }
    lines
}

/// Pieces split into words, carrying their look.
fn words(pieces: &[Piece]) -> Vec<Piece> {
    let mut out = Vec::new();
    for piece in pieces {
        let mut word = String::new();
        for character in piece.text.chars() {
            word.push(character);
            if character.is_whitespace() {
                out.push(Piece { text: std::mem::take(&mut word), ..piece.clone() });
            }
        }
        if !word.is_empty() {
            out.push(Piece { text: word, ..piece.clone() });
        }
    }
    out
}

fn self_advance(piece: &Piece, fonts: &Loaded) -> f32 {
    let mono = piece.mono || piece.code;
    piece.text.chars().map(|character| fonts.face(fonts.slot(character, mono)).advance(character, piece.size)).sum()
}

fn advance(piece: &Piece, fonts: &Loaded) -> f32 {
    self_advance(piece, fonts)
}

fn text_width(pieces: &[Piece], fonts: &Loaded) -> f32 {
    pieces.iter().map(|piece| self_advance(piece, fonts)).sum()
}

/// The `card` and `mnemonic` callouts are study material: the same violet as a note.
fn callout_color(kind: &str) -> Rgb {
    match kind {
        "note" | "abstract" | "summary" | "tldr" | "example" | "todo" | "embed" | "card" | "flashcard" | "mnemonic" => Rgb(0.26, 0.27, 0.64),
        "info" => Rgb(0.16, 0.34, 0.65),
        "tip" | "hint" | "success" | "check" | "done" => Rgb(0.04, 0.49, 0.34),
        "warning" | "caution" | "attention" => Rgb(0.65, 0.42, 0.03),
        "danger" | "error" | "bug" | "failure" | "fail" | "missing" => Rgb(0.64, 0.18, 0.18),
        "question" | "help" | "faq" => Rgb(0.51, 0.34, 0.86),
        "important" => Rgb(0.63, 0.19, 0.41),
        _ => Rgb(0.42, 0.45, 0.50),
    }
}

/// A colour mixed towards white: the callout's and the code block's background.
fn tint(color: Rgb, amount: f32) -> Rgb {
    Rgb(color.0 + (1.0 - color.0) * (1.0 - amount), color.1 + (1.0 - color.1) * (1.0 - amount), color.2 + (1.0 - color.2) * (1.0 - amount))
}

fn fill(color: Rgb, opacity: f32) -> Fill {
    Fill {
        paint: rgb::Color::new(channel(color.0), channel(color.1), channel(color.2)).into(),
        opacity: NormalizedF32::new(opacity).unwrap_or(NormalizedF32::ONE),
        rule: FillRule::NonZero,
    }
}

fn stroke(color: Rgb, width: f32) -> krilla::paint::Stroke {
    krilla::paint::Stroke { paint: rgb::Color::new(channel(color.0), channel(color.1), channel(color.2)).into(), width, ..Default::default() }
}

/// Writes `bytes` to `path` atomically: a sibling temporary file, then a rename.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("pdf.part");
    std::fs::write(&temporary, bytes).map_err(|error| format!("{}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("{}: {error}", path.display())
    })
}

/// A 0..1 channel as the 0..255 the backend takes.
fn channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
#[cfg(test)]
mod tests {
    use super::*;
    use localobsidian_md::preview;

    fn faces() -> Faces {
        Faces { regular: Face::new(epaint_default_fonts::UBUNTU_LIGHT), mono: Face::new(epaint_default_fonts::HACK_REGULAR), fallbacks: Vec::new() }
    }

    /// A scratch file the test removes.
    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("localobsidian-pdf-{name}-{}.pdf", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn blocks(markdown: &str) -> Vec<Block> {
        preview::blocks(markdown)
    }

    #[test]
    fn a_note_becomes_a_pdf_with_real_text() {
        let note = "# Sepse\n\nParágrafo com **negrito**, *itálico*, `código`, ==destaque== e [[Outra nota]].\n\n- um item\n- [x] feito\n- [ ] pendente\n\n> citação\n\n> [!warning] Cuidado\n> O corpo do aviso.\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n\n---\n";
        let path = scratch("note");
        let report = export(&blocks(note), "Sepse", &faces(), &path).expect("the export");
        assert!(report.pages >= 1);
        let bytes = std::fs::read(&path).expect("the file");
        assert_eq!(report.bytes, bytes.len());
        assert!(bytes.starts_with(b"%PDF-"), "a PDF file");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/ToUnicode"), "the glyphs keep a Unicode mapping (selectable text)");
        assert!(text.contains("/FontFile2") || text.contains("/FontFile"), "the font is embedded");
        assert!(text.contains(&format!("/Count {}", report.pages)), "the page tree says how many pages there are");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_long_note_is_paginated_and_nothing_is_drawn_in_the_margin() {
        let mut note = String::from("# Longa\n\n");
        for index in 1..=120 {
            note.push_str(&format!("Paragrafo {index} com uma frase suficientemente longa para ocupar a linha inteira da pagina.\n\n"));
        }
        let mut sheet = Sheet::new("Longa".to_string());
        sheet.blocks(&blocks(&note), MARGIN, CONTENT, &Loaded::new(&faces()).expect("the faces"));
        assert!(sheet.pages.len() > 2, "120 paragraphs do not fit on one page");
        for page in &sheet.pages {
            for op in &page.ops {
                let (top, bottom) = match op {
                    Op::Glyphs { at, size, .. } => (at.y - size, at.y),
                    Op::Rect { rect, .. } => (rect.top(), rect.bottom()),
                    Op::Line { from, to, .. } => (from.y.min(to.y), from.y.max(to.y)),
                };
                assert!(top >= MARGIN - 1.0, "something was drawn above the margin: {top}");
                assert!(bottom <= PAGE.1 - MARGIN + 1.0, "something was drawn past the margin: {bottom}");
            }
        }
    }

    #[test]
    fn the_glyph_ranges_cover_the_original_text() {
        // The ranges are what a reader uses to rebuild the string: they must tile the piece.
        let fonts = Loaded::new(&faces()).expect("the faces");
        let piece = Piece::plain("ação 日本", 10.0);
        let glyphs = glyphs_for(&piece.text, 0, Slot::Regular, piece.size, &fonts);
        let mut at = 0;
        for glyph in &glyphs {
            assert_eq!(glyph.text_range.start, at, "ranges are contiguous");
            at = glyph.text_range.end;
        }
        assert_eq!(at, piece.text.len(), "every byte of the piece has a glyph");
        assert!(glyphs.iter().all(|glyph| glyph.x_advance > 0.0), "each glyph advances");
    }

    #[test]
    fn an_empty_note_is_one_blank_page() {
        let path = scratch("empty");
        let report = export(&[], "", &faces(), &path).expect("the export");
        assert_eq!(report.pages, 1);
        assert!(report.bytes > 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_note_in_a_script_without_a_face_still_writes() {
        // No fallback face here: the characters have no glyph, and the page says so by drawing the
        // notdef box - the export must not fail, and the text is still there for a reader.
        let path = scratch("cjk");
        let report = export(&blocks("# 日本語\n\n本文です。\n"), "日本語", &faces(), &path).expect("the export");
        assert!(report.pages >= 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_tall_table_starts_a_new_page_instead_of_running_off_it() {
        let mut note = String::from("| A | B |\n|---|---|\n");
        for index in 1..=80 {
            note.push_str(&format!("| {index} | valor {index} |\n"));
        }
        let mut sheet = Sheet::new("Tabela".to_string());
        sheet.blocks(&blocks(&note), MARGIN, CONTENT, &Loaded::new(&faces()).expect("the faces"));
        assert!(sheet.pages.len() > 1, "80 rows do not fit on one page");
        for page in &sheet.pages {
            for op in &page.ops {
                if let Op::Glyphs { at, .. } = op {
                    assert!(at.y <= PAGE.1 - MARGIN + 1.0, "a table row was drawn past the margin: {}", at.y);
                }
            }
        }
    }

    #[test]
    fn nothing_is_drawn_past_the_right_margin() {
        // A table with a long cell is the case that used to spill over the page: the columns must
        // fit the measure, and no run may be drawn past it.
        let note = "| Coluna curta | Uma coluna com nome bem mais longo | N | Outra |\n|---|---|---|---|\n| a | texto medio na celula | 1 | 2 |\n";
        let mut sheet = Sheet::new("Tabela".to_string());
        sheet.blocks(&blocks(note), MARGIN, CONTENT, &Loaded::new(&faces()).expect("the faces"));
        let right = MARGIN + CONTENT;
        for page in &sheet.pages {
            for op in &page.ops {
                if let Op::Glyphs { at, glyphs, size, .. } = op {
                    let width: f32 = glyphs.iter().map(|glyph| glyph.x_advance * size).sum();
                    assert!(at.x + width <= right + 1.0, "a run ends at {} (margin {right}): {width:.0} wide from {}", at.x + width, at.x);
                }
            }
        }
    }
}
