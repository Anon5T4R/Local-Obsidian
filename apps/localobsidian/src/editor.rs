//! The note pane: the editor, the reading view, and the split that shows both.
//!
//! The editor is plain text on purpose - the file is Markdown, and a note is edited where it is
//! read. Its Markdown-specific edits (wraps, list continuation, indentation) come from
//! `localobsidian_editor`, and the keys that drive them live in [`crate::editor_keys`].
//!
//! The reading view draws the block model from `localobsidian_md::preview`: headings, lists,
//! quotes, callouts, tables, code, images from the vault, and links - a `[[wikilink]]` there is a
//! real link, and clicking it asks the app to open the note it names.

use egui::{Color32, RichText, TextStyle};

use localobsidian_md::links;
use localobsidian_md::preview::{Align, Block, Span};

use crate::images::Images;
use crate::tl;
use crate::vault_state::{VaultState, ViewMode};

/// What the user asked the note pane for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorAction {
    None,
    Save,
    /// Open the note a wikilink names.
    Navigate(String),
    /// Filter the sidebar by a `#tag` the reading view clicked.
    FilterTag(String),
    /// Flip the `n`-th `- [ ]` of the note (the reading view clicked its box).
    ToggleTask(usize),
}

/// How a run of spans is drawn: the block's size, plus what the inline markers added.
#[derive(Clone, Copy)]
struct Style {
    size: f32,
    strong: bool,
    italics: bool,
    strike: bool,
    /// `None` = the theme's text colour.
    color: Option<Color32>,
}

impl Style {
    fn new(size: f32) -> Self {
        Self { size, strong: false, italics: false, strike: false, color: None }
    }

    fn rich(self, text: impl Into<String>) -> RichText {
        let mut rich = RichText::new(text).size(self.size);
        if self.strong {
            rich = rich.strong();
        }
        if self.italics {
            rich = rich.italics();
        }
        if self.strike {
            rich = rich.strikethrough();
        }
        match self.color {
            Some(color) => rich.color(color),
            None => rich,
        }
    }

    fn strong(self) -> Self {
        Self { strong: true, ..self }
    }

    fn italic(self) -> Self {
        Self { italics: true, ..self }
    }

    fn struck(self) -> Self {
        Self { strike: true, ..self }
    }

    fn colored(self, color: Color32) -> Self {
        Self { color: Some(color), ..self }
    }
}

/// Draws the note pane.
pub fn show(ui: &mut egui::Ui, state: &mut VaultState, images: &mut Images) -> EditorAction {
    let Some(active) = state.active.clone() else {
        ui.vertical_centered(|ui| {
            ui.add_space(48.0);
            ui.label(RichText::new(tl!("No note is open")).weak());
            ui.label(RichText::new(tl!("Pick one in the sidebar, or create a new note.")).weak());
        });
        return EditorAction::None;
    };

    let mut action = EditorAction::None;
    header(ui, state, &active, &mut action);
    ui.separator();

    match state.view {
        ViewMode::Edit => action = editor_pane(ui, state),
        ViewMode::Read => action = reading_pane(ui, state, images),
        ViewMode::Split => {
            // A draggable divider, the width the user left it at. `default_size` only sizes the
            // first frame; after that egui remembers what the drag chose, so the panel is asked what
            // it used and the share is kept for the next run.
            let total = ui.available_width();
            let requested = (total * state.split).clamp(MIN_PANE, (total - MIN_PANE).max(MIN_PANE));
            let source =
                egui::Panel::left("split-source").resizable(true).default_size(requested).min_size(MIN_PANE).show(ui, |ui| editor_pane(ui, state));
            if total > 1.0 {
                state.split = (source.response.rect.width() / total).clamp(0.15, 0.85);
            }
            if source.inner != EditorAction::None {
                action = source.inner;
            }
            let from_reading = reading_pane(ui, state, images);
            if from_reading != EditorAction::None {
                action = from_reading;
            }
        }
    }
    action
}

/// The narrowest a pane of the split may be dragged: below this the note cannot be read.
const MIN_PANE: f32 = 180.0;

/// The source, with the Markdown keys and the wikilink popup wired.
fn editor_pane(ui: &mut egui::Ui, state: &mut VaultState) -> EditorAction {
    let mut action = EditorAction::None;
    let mut changed = false;
    let editor_id = ui.make_persistent_id("note-editor");

    // The command keys first: they are consumed before the widget draws, so a `Tab` cannot both
    // indent and insert a tab - and the completion popup's keys cannot reach the note.
    if let Some(key) = crate::editor_keys::pre(ui, &mut state.buffer, editor_id) {
        match key {
            crate::editor_keys::EditorKey::Changed => changed = true,
            crate::editor_keys::EditorKey::Save => action = EditorAction::Save,
        }
    }
    if let Some(popup) = crate::completions::take_keys(ui)
        && let Some(completion) = state.completion.clone()
    {
        match popup {
            crate::completions::PopupKey::Up => {
                let mut completion = completion;
                crate::completions::move_by(&mut completion, -1);
                state.completion = Some(completion);
            }
            crate::completions::PopupKey::Down => {
                let mut completion = completion;
                crate::completions::move_by(&mut completion, 1);
                state.completion = Some(completion);
            }
            crate::completions::PopupKey::Accept => {
                if let Some(name) = completion.selected() {
                    let (text, caret) = crate::completions::accept(&state.buffer, &completion, name);
                    state.buffer = text;
                    crate::editor_keys::place_caret(ui, &state.buffer, editor_id, caret);
                    changed = true;
                }
                state.completion = None;
            }
            crate::completions::PopupKey::Dismiss => state.completion = None,
        }
    }

    // A search result asked for a line: the caret goes there before the widget reads the state.
    if let Some(line) = state.pending_line.take() {
        let at = line_start_byte(&state.buffer, line);
        crate::editor_keys::place_caret(ui, &state.buffer, editor_id, at);
        state.view = ViewMode::Edit;
    }

    let mut caret_rect = None;
    // The source fills the pane: a box sized by `desired_rows` alone stopped a third of the way
    // down the window, and the rest was empty background.
    let row_height = ui.text_style_height(&TextStyle::Monospace).max(1.0);
    let rows = ((ui.available_height() / row_height).floor() as usize).saturating_sub(1).max(8);
    egui::ScrollArea::vertical().id_salt("editor-scroll").auto_shrink([false, false]).show(ui, |ui| {
        let output = egui::TextEdit::multiline(&mut state.buffer)
            .id(editor_id)
            .font(TextStyle::Monospace)
            .desired_width(f32::INFINITY)
            .desired_rows(rows)
            .code_editor()
            .show(ui);
        changed |= output.response.changed();
        // List continuation reads the newline the widget has just inserted.
        changed |= crate::editor_keys::post(ui, &mut state.buffer, editor_id);

        // The caret's rect, for the completion popup.
        if let Some(range) = output.cursor_range {
            let relative = output.galley.pos_from_cursor(range.primary);
            caret_rect = Some(relative.translate(output.galley_pos.to_vec2()));
        }
    });

    if changed {
        state.touch();
    }

    // The popup: what the caret is typing now, drawn under it.
    let caret = crate::editor_keys::caret_byte(ui, &state.buffer, editor_id);
    state.completion = crate::completions::for_caret(state, &state.buffer, caret);
    if let (Some(completion), Some(rect)) = (&state.completion, caret_rect) {
        let at = egui::pos2(rect.left(), rect.bottom() + 2.0);
        if let Some(index) = crate::completions::show(ui, completion, at) {
            let name = completion.candidates[index].clone();
            let (text, caret) = crate::completions::accept(&state.buffer, completion, &name);
            state.buffer = text;
            crate::editor_keys::place_caret(ui, &state.buffer, editor_id, caret);
            state.completion = None;
            state.touch();
        }
    }
    action
}

/// The byte offset where a 1-based line starts.
fn line_start_byte(text: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    let mut at = 0;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            at = index + 1;
            if text[..at].matches('\n').count() == line - 1 {
                break;
            }
        }
    }
    at
}

/// The reading view, scrolling to the section a link asked for.
fn reading_pane(ui: &mut egui::Ui, state: &mut VaultState, images: &mut Images) -> EditorAction {
    if !state.view.shows_reading() {
        return EditorAction::None;
    }
    state.refresh_preview();
    let section = state.pending_section.take();
    let vault: &VaultState = state;
    let blocks = vault.preview();
    let mut action = EditorAction::None;
    let mut target = section.as_deref();
    egui::ScrollArea::vertical().id_salt("reading-scroll").auto_shrink([false, false]).show(ui, |ui| {
        if let Some(hit) = show_blocks(ui, blocks, vault, images, &mut target) {
            action = match hit {
                Hit::Link(target) => EditorAction::Navigate(target),
                Hit::Tag(tag) => EditorAction::FilterTag(tag),
                Hit::Task(index) => EditorAction::ToggleTask(index),
            };
        }
    });
    action
}

fn header(ui: &mut egui::Ui, state: &mut VaultState, active: &std::path::Path, action: &mut EditorAction) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        let name = active.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
        let mut title = RichText::new(name).strong();
        if state.dirty {
            title = title.color(ui.visuals().warn_fg_color);
        }
        ui.label(title);
        if let Some(root) = &state.root {
            let relative = active.strip_prefix(root).unwrap_or(active).to_string_lossy().replace('\\', "/");
            ui.label(RichText::new(relative).small().weak());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Save belongs to the panes that can edit; in the reading view there is nothing to save
            // and the button only asks a question nobody asked.
            if state.view != ViewMode::Read && ui.add_enabled(state.dirty, egui::Button::new(tl!("Save"))).clicked() {
                *action = EditorAction::Save;
            }
            // The three panes, all visible: a single button that cycled left the modes hidden, and
            // the reading view looked like it was not rendering at all.
            let modes = egui::Frame::new().fill(ui.visuals().extreme_bg_color).corner_radius(6).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
                // The frame inherits the right-to-left layout of the header; the modes inside it
                // read left to right, in the order they escalate.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    for (mode, label) in [(ViewMode::Edit, tl!("Edit")), (ViewMode::Split, tl!("Split")), (ViewMode::Read, tl!("Read"))] {
                        if ui.selectable_label(state.view == mode, RichText::new(label).small()).clicked() {
                            state.view = mode;
                        }
                    }
                });
            });
            modes.response.on_hover_text(tl!("Switch between the editor, the reading view and both"));
        });
    });
}

/// What the reading view hit: a link to a note (its section is part of the target), a tag, or a
/// checkbox.
enum Hit {
    Link(String),
    /// A `#tag` of the note: the sidebar filters by it.
    Tag(String),
    /// A task checkbox of the reading view: the ordinal of the checkbox in the note.
    Task(usize),
}

/// Draws blocks, returning what the user clicked. `target` is the section a link asked for: the
/// first heading that matches it is scrolled to and the target is cleared.
fn show_blocks(ui: &mut egui::Ui, blocks: &[Block], state: &VaultState, images: &mut Images, target: &mut Option<&str>) -> Option<Hit> {
    let mut hit = None;
    for block in blocks {
        let found = show_block(ui, block, state, images, target);
        if found.is_some() {
            hit = found;
        }
    }
    hit
}

fn show_block(ui: &mut egui::Ui, block: &Block, state: &VaultState, images: &mut Images, target: &mut Option<&str>) -> Option<Hit> {
    match block {
        Block::Heading { level, spans } => {
            ui.add_space(6.0);
            let mut style = Style::new(heading_size(*level));
            style.strong = true;
            let text: String = spans.iter().map(Span::plain).collect();
            let drawn = ui.vertical(|ui| show_spans(ui, spans, state, style));
            if let Some(section) = *target
                && localobsidian_md::outline::section_matches(section, &text)
            {
                ui.scroll_to_rect(drawn.response.rect, Some(egui::Align::TOP));
                *target = None;
            }
            ui.add_space(2.0);
            drawn.inner
        }
        Block::Paragraph(spans) => {
            let hit = show_spans(ui, spans, state, Style::new(14.5));
            ui.add_space(4.0);
            hit
        }
        Block::List { ordered, items, tasks } => {
            let mut hit = None;
            for (index, item) in items.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    match tasks.get(index).copied().flatten() {
                        // A checklist draws a real box: the note's source is what a click edits.
                        Some(task) => {
                            let mut done = task.done;
                            if ui.add(egui::Checkbox::without_text(&mut done)).on_hover_text(tl!("Tick it in the note")).clicked() {
                                hit = Some(Hit::Task(task.index));
                            }
                        }
                        None => {
                            let marker = if *ordered { format!("{}.", index + 1) } else { "•".to_string() };
                            ui.label(RichText::new(marker).weak());
                        }
                    }
                    for span in item {
                        if let Some(found) = show_span(ui, span, state, Style::new(14.5)) {
                            hit = Some(found);
                        }
                    }
                });
            }
            ui.add_space(4.0);
            hit
        }
        Block::Code { language, text } => {
            if !language.is_empty() {
                ui.label(RichText::new(language).small().weak());
            }
            egui::Frame::new().fill(ui.visuals().code_bg_color).inner_margin(8.0).corner_radius(4.0).show(ui, |ui| {
                ui.label(RichText::new(text).monospace());
            });
            ui.add_space(4.0);
            None
        }
        Block::Quote(inner) => {
            let weak = ui.visuals().weak_text_color();
            let quoted = ui.indent(ui.scope_id().with("quote"), |ui| {
                ui.visuals_mut().override_text_color = Some(weak);
                show_blocks(ui, inner, state, images, target)
            });
            let rect = quoted.response.rect;
            if rect.height() > 0.0 {
                let bar = egui::Rect::from_min_size(egui::pos2(rect.left() - 10.0, rect.top()), egui::vec2(3.0, rect.height()));
                // The one place the note's own colour shows without a callout type: a quote is the
                // author's voice, and the accent marks it without shouting.
                ui.painter().rect_filled(bar, 1.0, crate::theme::accent(ui.visuals().dark_mode).gamma_multiply(0.8));
            }
            quoted.inner
        }
        Block::Callout { kind, title, body } => show_callout(ui, kind, title, body, state, images, target),
        Block::Table { headers, rows, aligns } => show_table(ui, headers, rows, aligns, state),
        Block::Image { destination, alt } => {
            show_image(ui, destination, alt, state, images);
            None
        }
        Block::Rule => {
            ui.separator();
            None
        }
    }
}

/// `> [!kind] Título`: a coloured box, the title in bold, the body drawn as blocks.
fn show_callout(
    ui: &mut egui::Ui,
    kind: &str,
    title: &str,
    body: &[Block],
    state: &VaultState,
    images: &mut Images,
    target: &mut Option<&str>,
) -> Option<Hit> {
    let color = callout_color(kind);
    let mut hit = None;
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.55)))
        .inner_margin(10.0)
        .corner_radius(6.0)
        .show(ui, |ui| {
            ui.label(RichText::new(title).strong().color(color));
            ui.add_space(2.0);
            if let Some(found) = show_blocks(ui, body, state, images, target) {
                hit = Some(found);
            }
        });
    ui.add_space(6.0);
    hit
}

/// The colour a callout type is drawn in: the types the ecosystem shares, and a neutral fallback.
fn callout_color(kind: &str) -> Color32 {
    match kind {
        "note" | "abstract" | "summary" | "tldr" | "example" | "todo" | "embed" => Color32::from_rgb(99, 102, 241),
        "info" => Color32::from_rgb(59, 130, 246),
        "tip" | "hint" | "success" | "check" | "done" => Color32::from_rgb(16, 185, 129),
        "warning" | "caution" | "attention" => Color32::from_rgb(245, 158, 11),
        "danger" | "error" | "bug" | "failure" | "fail" | "missing" => Color32::from_rgb(239, 68, 68),
        "question" | "help" | "faq" => Color32::from_rgb(139, 92, 246),
        "important" => Color32::from_rgb(236, 72, 153),
        _ => Color32::from_rgb(107, 114, 128),
    }
}

/// Room around a cell's text.
const TABLE_PAD: f32 = 8.0;
/// No column is squeezed below this: a word still fits, so a line never breaks inside one.
const TABLE_MIN_COL: f32 = 72.0;
/// The tallest a single row may get; a table is a table, not a page.
const TABLE_MAX_ROW: f32 = 240.0;

/// A table: header band, alternating rows, gridlines and columns sized to their content and to the
/// page.
///
/// `egui::Grid` sizes its columns from what the cells *measure* while laying them out, and a cell
/// laid out inside a `with_layout` measures as almost nothing: the grid fell back to its minimum and
/// the table turned into a column of tall slivers. Measuring the text here first, then painting the
/// bands and drawing the cells into rects of the width that came out, is what fixes both.
fn show_table(ui: &mut egui::Ui, headers: &[Vec<Span>], rows: &[Vec<Vec<Span>>], aligns: &[Align], state: &VaultState) -> Option<Hit> {
    let columns = headers.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return None;
    }
    let body = Style::new(14.0);
    let head = Style::new(14.0).strong();
    let available = ui.available_width().max(TABLE_MIN_COL * columns as f32);

    // What each column would like: its widest cell, measured without wrapping.
    let mut natural = vec![TABLE_MIN_COL; columns];
    for (column, cell) in headers.iter().enumerate() {
        natural[column] = natural[column].max(text_width(ui, cell, head) + 2.0 * TABLE_PAD);
    }
    for row in rows {
        for (column, cell) in row.iter().enumerate() {
            if let Some(slot) = natural.get_mut(column) {
                *slot = (*slot).max(text_width(ui, cell, body) + 2.0 * TABLE_PAD);
            }
        }
    }
    let widths = column_widths(available, &natural);

    // Each row's height, from the tallest cell wrapped at the width it will get.
    let heights: Vec<f32> = std::iter::once(headers)
        .chain(rows.iter().map(|row| row.as_slice()))
        .enumerate()
        .map(|(index, cells)| {
            let style = if index == 0 { head } else { body };
            row_height(ui, cells, &widths, style, TABLE_PAD).min(TABLE_MAX_ROW)
        })
        .collect();
    let total_width: f32 = widths.iter().sum();
    let total_height: f32 = heights.iter().sum();
    let table = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(total_width, total_height));
    ui.allocate_rect(table, egui::Sense::hover());

    // The bands and the lines first, so the text sits on top of them.
    let line = ui.visuals().weak_text_color().gamma_multiply(0.4);
    let painter = ui.painter();
    let mut y = table.top();
    for (index, height) in heights.iter().enumerate() {
        // The header band, then every other row a shade lighter.
        if index == 0 {
            painter.rect_filled(
                egui::Rect::from_min_size(egui::pos2(table.left(), y), egui::vec2(total_width, *height)),
                0.0,
                ui.visuals().faint_bg_color,
            );
        } else if index % 2 == 0 {
            painter.rect_filled(
                egui::Rect::from_min_size(egui::pos2(table.left(), y), egui::vec2(total_width, *height)),
                0.0,
                ui.visuals().faint_bg_color.gamma_multiply(0.55),
            );
        }
        y += height;
        painter.line_segment([egui::pos2(table.left(), y), egui::pos2(table.right(), y)], egui::Stroke::new(1.0, line));
    }
    let mut x = table.left();
    for width in &widths {
        x += width;
        painter.line_segment([egui::pos2(x, table.top()), egui::pos2(x, table.bottom())], egui::Stroke::new(1.0, line));
    }
    painter.rect_stroke(table, 0.0, egui::Stroke::new(1.0, line), egui::StrokeKind::Inside);

    // The content, cell by cell, into the rect that was measured for it.
    let mut hit = None;
    let mut y = table.top();
    for (index, cells) in std::iter::once(headers).chain(rows.iter().map(|row| row.as_slice())).enumerate() {
        let style = if index == 0 { head } else { body };
        let height = heights[index];
        let mut x = table.left();
        for (column, cell) in cells.iter().enumerate() {
            let width = widths.get(column).copied().unwrap_or(TABLE_MIN_COL);
            let rect = egui::Rect::from_min_size(
                egui::pos2(x + TABLE_PAD, y + TABLE_PAD),
                egui::vec2((width - 2.0 * TABLE_PAD).max(8.0), (height - 2.0 * TABLE_PAD).max(8.0)),
            );
            let align = aligns.get(column).copied().unwrap_or_default();
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(cell_layout(Some(align))), |ui| {
                if let Some(found) = show_spans(ui, cell, state, style) {
                    hit = Some(found);
                }
            });
            x += width;
        }
        y += height;
    }
    ui.add_space(6.0);
    hit
}

/// The width of a cell's text as one line.
fn text_width(ui: &egui::Ui, spans: &[Span], style: Style) -> f32 {
    let text: String = spans.iter().map(Span::plain).collect();
    let font = egui::FontId::proportional(style.size);
    ui.painter().layout_no_wrap(text, font, egui::Color32::WHITE).size().x
}

/// The height a cell needs when wrapped at `width`, plus the padding above and below.
fn row_height(ui: &egui::Ui, cells: &[Vec<Span>], widths: &[f32], style: Style, pad: f32) -> f32 {
    let font = egui::FontId::proportional(style.size);
    let mut tallest: f32 = ui.text_style_height(&egui::TextStyle::Body);
    for (column, cell) in cells.iter().enumerate() {
        let width = widths.get(column).copied().unwrap_or(TABLE_MIN_COL).max(8.0) - 2.0 * pad;
        let text: String = cell.iter().map(Span::plain).collect();
        let height = ui.painter().layout(text, font.clone(), egui::Color32::WHITE, width).size().y;
        tallest = tallest.max(height);
    }
    tallest + 2.0 * pad
}

/// How wide each column is: what it asked for, grown to fill the page when there is room, shrunk
/// but never below [`MIN_COL`] when there is not.
fn column_widths(available: f32, natural: &[f32]) -> Vec<f32> {
    let asked: f32 = natural.iter().sum();
    if asked <= available && asked > 0.0 {
        let slack = available - asked;
        return natural.iter().map(|width| width + slack * (width / asked)).collect();
    }
    let mut widths = natural.to_vec();
    // Shrink proportionally in a few passes: one pass overshoots, because taking the same share from
    // a column that already hit the floor would make the others too narrow.
    for _ in 0..6 {
        let total: f32 = widths.iter().sum();
        if total <= available {
            break;
        }
        let scale = (available / total).min(1.0);
        for width in &mut widths {
            *width = (*width * scale).max(TABLE_MIN_COL);
        }
    }
    widths
}

fn cell_layout(align: Option<Align>) -> egui::Layout {
    let align = match align.unwrap_or_default() {
        Align::Left => egui::Align::Min,
        Align::Center => egui::Align::Center,
        Align::Right => egui::Align::Max,
    };
    egui::Layout::left_to_right(egui::Align::Center).with_main_align(align)
}

/// An image of the vault; when the file is missing or unreadable, its name stays visible.
fn show_image(ui: &mut egui::Ui, destination: &str, alt: &[Span], state: &VaultState, images: &mut Images) {
    let alt_text: String = alt.iter().map(Span::plain).collect();
    let path = state.attachment_path(destination);
    let texture = path.as_deref().and_then(|path| images.texture(ui.ctx(), path));
    match (path, texture) {
        (Some(path), Some((texture, [width, height]))) => {
            let available = ui.available_width().max(64.0);
            let scale = (available / width as f32).min(1.0);
            let size = egui::vec2(width as f32 * scale, height as f32 * scale);
            let response = ui.add(egui::Image::new(&texture).fit_to_exact_size(size));
            if !alt_text.is_empty() {
                response.on_hover_text(alt_text);
            }
            ui.label(RichText::new(path.file_name().unwrap_or_default().to_string_lossy().to_string()).small().weak());
        }
        _ => {
            let label = if alt_text.is_empty() { destination.to_string() } else { alt_text };
            ui.label(RichText::new(label).weak().italics());
        }
    }
    ui.add_space(4.0);
}

fn heading_size(level: u8) -> f32 {
    match level {
        1 => 22.0,
        2 => 19.0,
        3 => 17.0,
        4 => 15.5,
        _ => 14.5,
    }
}

/// Draws inline spans in one wrapped run.
fn show_spans(ui: &mut egui::Ui, spans: &[Span], state: &VaultState, style: Style) -> Option<Hit> {
    let mut hit = None;
    ui.horizontal_wrapped(|ui| {
        // Spans sit next to each other the way words do: the default item spacing would open a
        // visible gap around every link and every tag.
        ui.spacing_mut().item_spacing.x = 0.0;
        for span in spans {
            if let Some(found) = show_span(ui, span, state, style) {
                hit = Some(found);
            }
        }
    });
    hit
}

fn show_span(ui: &mut egui::Ui, span: &Span, state: &VaultState, style: Style) -> Option<Hit> {
    match span {
        Span::Text(text) => {
            ui.label(style.rich(drawable(ui, text, style.size)));
            None
        }
        Span::Code(text) => {
            ui.label(style.rich(text.clone()).monospace().background_color(ui.visuals().code_bg_color));
            None
        }
        Span::Strong(inner) => nested(ui, inner, state, style.strong()),
        Span::Emphasis(inner) => nested(ui, inner, state, style.italic()),
        Span::Strike(inner) => nested(ui, inner, state, style.struck()),
        Span::Tag(tag) => {
            let accent = crate::theme::accent(ui.visuals().dark_mode);
            let pill = egui::Frame::new()
                .fill(accent.gamma_multiply(0.18))
                .corner_radius(9)
                .inner_margin(egui::Margin { left: 6, right: 6, top: 1, bottom: 1 })
                .show(ui, |ui| ui.label(style.rich(format!("#{tag}")).color(accent)));
            let response = pill.response.interact(egui::Sense::click());
            if response.clicked() {
                return Some(Hit::Tag(tag.clone()));
            }
            None
        }
        Span::Link { spans, destination } => show_link(ui, spans, destination, state, style),
    }
}

/// The text as the font can actually draw it.
///
/// The app ships no emoji font (a decision, not an oversight): the fonts it does ship cover some
/// pictographs and not others, so a note written with emojis would show some and draw a hollow box
/// for the rest. A box in the middle of a title is worse than a shorter title, so a pictograph the
/// font has no glyph for is left out of the *drawn* text - the note keeps it, the editor shows it,
/// the reading view simply does not draw a box.
fn drawable(ui: &egui::Ui, text: &str, size: f32) -> String {
    if !text.chars().any(is_pictograph) {
        return text.to_string();
    }
    let font = egui::FontId::proportional(size);
    ui.ctx().fonts_mut(|fonts| text.chars().filter(|c| !is_pictograph(*c) || fonts.has_glyph(&font, *c)).collect())
}

/// The ranges a note uses for pictographs: arrows, symbols and the emoji planes.
fn is_pictograph(c: char) -> bool {
    matches!(
        u32::from(c),
        0x200D | 0x20E3 | 0xFE0F | 0x2190..=0x21FF | 0x2300..=0x23FF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x1F000..=0x1FAFF
    )
}

/// A nested run (bold inside italic), drawn into the run already open.
fn nested(ui: &mut egui::Ui, spans: &[Span], state: &VaultState, style: Style) -> Option<Hit> {
    let mut hit = None;
    for span in spans {
        if let Some(found) = show_span(ui, span, state, style) {
            hit = Some(found);
        }
    }
    hit
}

fn show_link(ui: &mut egui::Ui, spans: &[Span], destination: &str, state: &VaultState, style: Style) -> Option<Hit> {
    let text: String = spans.iter().map(Span::plain).collect();
    if let Some(target) = links::from_url(destination) {
        let parsed = localobsidian_md::parse_target(&target);
        let resolves = state.resolve(parsed.note).is_some();
        let color = if resolves { ui.visuals().hyperlink_color } else { ui.visuals().weak_text_color() };
        let response = ui.add(egui::Label::new(style.colored(color).rich(text)).sense(egui::Sense::click()));
        if resolves && response.clicked() {
            return Some(Hit::Link(parsed.note.to_string()));
        }
        return None;
    }
    if destination.starts_with("http://") || destination.starts_with("https://") {
        ui.hyperlink_to(style.rich(text), destination);
        return None;
    }
    ui.label(style.rich(text));
    None
}
