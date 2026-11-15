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
            ui.columns(2, |columns| {
                let from_editor = editor_pane(&mut columns[0], state);
                if from_editor != EditorAction::None {
                    action = from_editor;
                }
                let from_reading = reading_pane(&mut columns[1], state, images);
                if from_reading != EditorAction::None {
                    action = from_reading;
                }
            });
        }
    }
    action
}

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
    egui::ScrollArea::vertical().id_salt("editor-scroll").auto_shrink([false, false]).show(ui, |ui| {
        let output = egui::TextEdit::multiline(&mut state.buffer)
            .id(editor_id)
            .font(TextStyle::Monospace)
            .desired_width(f32::INFINITY)
            .desired_rows(24)
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
            if ui.add_enabled(state.dirty, egui::Button::new(tl!("Save"))).clicked() {
                *action = EditorAction::Save;
            }
            let label = match state.view {
                ViewMode::Edit => tl!("Edit"),
                ViewMode::Read => tl!("Read"),
                ViewMode::Split => tl!("Split"),
            };
            if ui.selectable_label(false, label).on_hover_text(tl!("Switch between the editor, the reading view and both")).clicked() {
                state.view = state.view.next();
            }
        });
    });
}

/// What the reading view hit: a link to a note (its section is part of the target).
enum Hit {
    Link(String),
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
                ui.painter().rect_filled(bar, 1.0, weak);
            }
            quoted.inner
        }
        Block::Callout { kind, title, body } => show_callout(ui, kind, title, body, state, images, target),
        Block::Table { headers, rows, aligns } => {
            show_table(ui, headers, rows, aligns, state);
            None
        }
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

/// A table: header row in bold, one line per row, columns aligned as the source asked.
fn show_table(ui: &mut egui::Ui, headers: &[Vec<Span>], rows: &[Vec<Vec<Span>>], aligns: &[Align], state: &VaultState) {
    egui::Grid::new(ui.next_auto_id()).striped(true).min_col_width(60.0).show(ui, |ui| {
        for (column, cell) in headers.iter().enumerate() {
            let style = Style::new(14.0).strong();
            ui.with_layout(cell_layout(aligns.get(column).copied()), |ui| {
                let _ = show_spans(ui, cell, state, style);
            });
        }
        ui.end_row();
        for row in rows {
            for (column, cell) in row.iter().enumerate() {
                ui.with_layout(cell_layout(aligns.get(column).copied()), |ui| {
                    let _ = show_spans(ui, cell, state, Style::new(14.0));
                });
            }
            ui.end_row();
        }
    });
    ui.add_space(6.0);
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
            ui.label(style.rich(text.clone()));
            None
        }
        Span::Code(text) => {
            ui.label(style.rich(text.clone()).monospace().background_color(ui.visuals().code_bg_color));
            None
        }
        Span::Strong(inner) => nested(ui, inner, state, style.strong()),
        Span::Emphasis(inner) => nested(ui, inner, state, style.italic()),
        Span::Strike(inner) => nested(ui, inner, state, style.struck()),
        Span::Link { spans, destination } => show_link(ui, spans, destination, state, style),
    }
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
