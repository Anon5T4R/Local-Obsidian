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
    /// Walk the history one note back.
    Back,
    /// Walk the history one note forward.
    Forward,
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

    // The pane's own action, if it has one. It must not overwrite what the header already asked
    // for: `action = pane(..)` silently ate the header's Save when the header learned about it,
    // and would have eaten the two arrows the same way.
    let from_pane = match state.view {
        ViewMode::Edit => editor_pane(ui, state),
        ViewMode::Read => reading_pane(ui, state, images),
        ViewMode::Split => {
            // A draggable divider, the width the user left it at. `default_size` only sizes the
            // first frame; after that egui remembers what the drag chose, so the panel is asked what
            // it used and the share is kept for the next run.
            let total = ui.available_width();
            let requested = (total * state.split).clamp(MIN_PANE, (total - MIN_PANE).max(MIN_PANE));
            let source = egui::Panel::left("split-source")
                .resizable(true)
                .default_size(requested)
                .min_size(MIN_PANE)
                // The reading pane keeps its own room even when the divider is dragged all the way:
                // a split with one pane at 100% is not a split.
                .max_size((total - MIN_PANE).max(MIN_PANE))
                .show(ui, |ui| editor_pane(ui, state));
            if total > 1.0 {
                state.split = (source.response.rect.width() / total).clamp(0.15, 0.85);
            }
            let from_reading = reading_pane(ui, state, images);
            if from_reading != EditorAction::None { from_reading } else { source.inner }
        }
    };
    if from_pane != EditorAction::None {
        action = from_pane;
    }
    action
}

/// The narrowest a pane of the split may be dragged: below this the note cannot be read.
const MIN_PANE: f32 = 180.0;

/// The reading view's body size before the zoom: the measure every other block is a multiple of.
const BODY_SIZE: f32 = 15.0;

/// A piece of markdown drawn by the reading view, outside a note: the front and the back of a
/// review card, which are written in the same syntax as the note they came from.
///
/// `images` is the app's cache, so a card that embeds a picture costs nothing extra.
pub fn markdown(ui: &mut egui::Ui, state: &VaultState, images: &mut Images, text: &str, size: f32) {
    let prepared = localobsidian_md::preview::prepare(text);
    let blocks = localobsidian_md::preview::blocks(&prepared);
    let mut target: Option<&str> = None;
    show_blocks(ui, &blocks, state, images, &mut target, size / BODY_SIZE);
}

/// A highlighter for the source pane: fences, headings, quotes, tags and wikilinks get a colour, so
/// the Markdown reads as structure and not as one grey wall. It is a *source* view: nothing is hidden,
/// only coloured.
fn markdown_layouter(ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32) -> std::sync::Arc<egui::Galley> {
    let visuals = ui.visuals();
    let accent = crate::theme::accent(visuals.dark_mode);
    let muted = visuals.weak_text_color();
    let font = egui::FontId::monospace(13.0);
    let plain = egui::TextFormat { font_id: font.clone(), color: visuals.text_color(), ..Default::default() };
    let heading = egui::TextFormat { font_id: font.clone(), color: accent, ..Default::default() };
    let dim = egui::TextFormat { font_id: font.clone(), color: muted, ..Default::default() };
    let mark = egui::TextFormat { font_id: font.clone(), color: accent, background: accent.gamma_multiply(0.14), ..Default::default() };

    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let mut in_fence = false;
    for line in text.as_str().split_inclusive('\n') {
        let trimmed = line.trim_start();
        let fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if fence {
            in_fence = !in_fence;
        }
        let line_format = if fence || in_fence {
            &dim
        } else if trimmed.starts_with('#') {
            &heading
        } else if trimmed.starts_with('>') {
            &dim
        } else {
            &plain
        };

        // Inline pieces worth a colour, on a line that is not inside a fence: `[[link]]`, `#tag`,
        // `==highlight==`.
        if in_fence || fence {
            job.append(line, 0.0, line_format.clone());
            continue;
        }
        let mut rest = line;
        while !rest.is_empty() {
            let next = ["[[", "#", "=="].iter().filter_map(|open| rest.find(open).map(|at| (at, *open))).min_by_key(|(at, _)| *at);
            let Some((at, open)) = next else {
                job.append(rest, 0.0, line_format.clone());
                break;
            };
            if at > 0 {
                job.append(&rest[..at], 0.0, line_format.clone());
            }
            let close = match open {
                "[[" => Some("]]"),
                "==" => Some("=="),
                _ => None,
            };
            let after = &rest[at + open.len()..];
            let end = match close {
                Some(close) => after.find(close).map(|end| end + close.len()).unwrap_or(after.len()),
                // A tag runs while the characters can be part of one.
                None => after.find(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '/'))).unwrap_or(after.len()),
            };
            let piece_len = open.len() + end;
            job.append(&rest[at..at + piece_len], 0.0, mark.clone());
            rest = &rest[at + piece_len..];
        }
    }
    ui.ctx().fonts_mut(|fonts| fonts.layout_job(job))
}

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
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| markdown_layouter(ui, text, wrap);
        let output = egui::TextEdit::multiline(&mut state.buffer)
            .id(editor_id)
            .font(TextStyle::Monospace)
            // The width of the pane, not of the window: a widget that asks for `f32::INFINITY`
            // also widens the panel that holds it, and the split's source would grow over the note.
            .desired_width(ui.available_width())
            .desired_rows(rows)
            .code_editor()
            .layouter(&mut layouter)
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

/// The widest the reading column gets, before the zoom: a line of 150 characters is a wall, and 820
/// px is the measure a screen of text has had since print.
const READING_WIDTH: f32 = 820.0;

/// The reading view, scrolling to the section a link asked for.
fn reading_pane(ui: &mut egui::Ui, state: &mut VaultState, images: &mut Images) -> EditorAction {
    if !state.view.shows_reading() {
        return EditorAction::None;
    }
    state.refresh_preview();
    // The count comes from the note as it is now, so the bar can say "3 / 12" on the frame the
    // query changes; the matches themselves are drawn below, in reading order.
    let total = if crate::find::is_searching(&state.find) {
        let query = state.find.query.trim().to_string();
        count_matches(state.preview(), &query)
    } else {
        0
    };
    state.find.total = total;
    state.find.seen.set(0);
    if state.find.open {
        find_bar(ui, state);
        ui.separator();
    }
    let section = state.pending_section.take();
    let zoom = state.zoom;
    let vault: &VaultState = state;
    let blocks = vault.preview();
    let fields = vault.frontmatter();
    let mut action = EditorAction::None;
    let mut target = section.as_deref();
    egui::ScrollArea::vertical().id_salt("reading-scroll").auto_shrink([false, false]).show(ui, |ui| {
        // The column is centred with generous margins: the text is what the window is for.
        let column = (READING_WIDTH * zoom).min(ui.available_width());
        let pad = ((ui.available_width() - column) / 2.0).max(0.0);
        ui.horizontal_top(|ui| {
            ui.add_space(pad);
            ui.vertical(|ui| {
                ui.set_max_width(column);
                if !fields.is_empty() {
                    properties(ui, fields);
                }
                if let Some(hit) = show_blocks(ui, blocks, vault, images, &mut target, zoom) {
                    action = match hit {
                        Hit::Link(target) => EditorAction::Navigate(target),
                        Hit::Tag(tag) => EditorAction::FilterTag(tag),
                        Hit::Task(index) => EditorAction::ToggleTask(index),
                    };
                }
            });
        });
    });
    action
}

/// The find bar above the reading view: the query, how many matches, and the two arrows.
fn find_bar(ui: &mut egui::Ui, state: &mut VaultState) {
    ui.horizontal(|ui| {
        let field = ui.add(egui::TextEdit::singleline(&mut state.find.query).hint_text(tl!("Find in this note")).desired_width(200.0));
        if state.find.focus {
            field.request_focus();
            state.find.focus = false;
        }
        if field.changed() {
            // A new query starts at the first match.
            state.find.current = 0;
        }
        if field.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            if ui.input(|input| input.modifiers.shift) {
                state.find.previous();
            } else {
                state.find.next();
            }
        }
        let total = state.find.total;
        let shown = if total == 0 { 0 } else { state.find.current.min(total - 1) + 1 };
        ui.label(egui::RichText::new(format!("{shown} / {total}")).small().weak());
        if chevron_button(ui, total > 0, true, tl!("Previous match")).clicked() {
            state.find.previous();
        }
        if chevron_button(ui, total > 0, false, tl!("Next match")).clicked() {
            state.find.next();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("x").on_hover_text(tl!("Close the find bar")).clicked() {
                state.find.close();
            }
        });
    });
}

/// How many times `needle` appears in the prose the reading view draws.
///
/// The same walk as the drawing: text runs (and the emphasis/link spans around them), list items and
/// the bodies of quotes and callouts. Code blocks, tables and tags are drawn by their own widgets and
/// are not searched.
fn count_matches(blocks: &[Block], needle: &str) -> usize {
    fn in_spans(list: &[Span], needle: &str) -> usize {
        list.iter()
            .map(|span| match span {
                Span::Text(text) => crate::find::ranges(text, needle).len(),
                Span::Strong(inner) | Span::Emphasis(inner) | Span::Strike(inner) => in_spans(inner, needle),
                Span::Link { spans, .. } => in_spans(spans, needle),
                _ => 0,
            })
            .sum()
    }
    blocks
        .iter()
        .map(|block| match block {
            Block::Heading { spans, .. } | Block::Paragraph(spans) => in_spans(spans, needle),
            Block::List { items, .. } => items.iter().map(|item| in_spans(item, needle)).sum(),
            Block::Quote(inner) | Block::Callout { body: inner, .. } => count_matches(inner, needle),
            _ => 0,
        })
        .sum()
}

/// The note's own metadata, as a card at the top: `tipo: sistema`, `aliases: Lacunas, Gap`. The block
/// is what tells a note apart from a page, and the reading view used to drop it silently.
fn properties(ui: &mut egui::Ui, fields: &[(String, String)]) {
    let accent = crate::theme::accent(ui.visuals().dark_mode);
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(egui::Stroke::new(1.0, ui.visuals().weak_text_color().gamma_multiply(0.35)))
        .corner_radius(8)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            for (key, value) in fields {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
                    ui.label(egui::RichText::new(key).small().weak());
                    // A list is a row of chips: one per alias, one per tag.
                    let chips = matches!(key.as_str(), "tags" | "tag" | "aliases" | "alias");
                    if chips && !value.is_empty() {
                        for item in value.split(',').map(str::trim).filter(|item| !item.is_empty()) {
                            let label = if matches!(key.as_str(), "tags" | "tag") { format!("#{item}") } else { item.to_string() };
                            egui::Frame::new()
                                .fill(accent.gamma_multiply(0.16))
                                .corner_radius(9)
                                .inner_margin(egui::Margin { left: 7, right: 7, top: 1, bottom: 1 })
                                .show(ui, |ui| ui.label(egui::RichText::new(label).small().color(accent)));
                        }
                    } else if !value.is_empty() {
                        ui.label(egui::RichText::new(value).small());
                    }
                });
            }
        });
    ui.add_space(14.0);
}

/// One of the two history chevrons, drawn with two lines.
///
/// The fonts the app embeds (egui's Latin face plus the craft-fonts CJK faces) have no `‹`/`›`, and
/// an arrow that renders as a box is worse than no arrow: the glyph is painted, not typed.
fn chevron_button(ui: &mut egui::Ui, enabled: bool, left: bool, tooltip: &str) -> egui::Response {
    let size = egui::vec2(20.0, 20.0);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let response = ui.allocate_response(size, sense);
    // Named for the accessibility tree (and for the tests): the arrow itself is paint, and paint
    // has no name.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, tooltip));
    let rect = response.rect;
    let visuals = ui.visuals();
    if enabled && (response.hovered() || response.is_pointer_button_down_on()) {
        let fill = visuals.widgets.hovered.bg_fill.gamma_multiply(0.6);
        ui.painter().rect_filled(rect.shrink(1.0), 5.0, fill);
    }
    let color = if enabled && response.hovered() {
        visuals.strong_text_color()
    } else if enabled {
        visuals.text_color()
    } else {
        visuals.weak_text_color().gamma_multiply(0.55)
    };
    let stroke = egui::Stroke::new(1.6, color);
    let centre = rect.center();
    let (tip, back) = if left { (-3.0, 3.0) } else { (3.0, -3.0) };
    let points = [egui::pos2(centre.x + back, centre.y - 5.0), egui::pos2(centre.x + tip, centre.y), egui::pos2(centre.x + back, centre.y + 5.0)];
    ui.painter().add(egui::Shape::line(points.to_vec(), stroke));
    if enabled { response.on_hover_text(tooltip) } else { response }
}

fn header(ui: &mut egui::Ui, state: &mut VaultState, active: &std::path::Path, action: &mut EditorAction) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        // The two arrows sit at the left, where a browser puts them and where the hand that just
        // came from the tree already is: they walk the notes, not the app.
        let can_back = state.history.can_go_back();
        let can_forward = state.history.can_go_forward();
        if chevron_button(ui, can_back, true, tl!("Back")).clicked() {
            *action = EditorAction::Back;
        }
        if chevron_button(ui, can_forward, false, tl!("Forward")).clicked() {
            *action = EditorAction::Forward;
        }
        ui.add_space(6.0);
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
fn show_blocks(ui: &mut egui::Ui, blocks: &[Block], state: &VaultState, images: &mut Images, target: &mut Option<&str>, zoom: f32) -> Option<Hit> {
    let mut hit = None;
    for block in blocks {
        let found = show_block(ui, block, state, images, target, zoom);
        if found.is_some() {
            hit = found;
        }
    }
    hit
}

fn show_block(ui: &mut egui::Ui, block: &Block, state: &VaultState, images: &mut Images, target: &mut Option<&str>, zoom: f32) -> Option<Hit> {
    match block {
        Block::Heading { level, spans } => {
            // The steps are wide enough to read as a hierarchy, and the space above a heading is
            // what makes a long note skimmable: 26 px above a chapter, 18 above a section.
            ui.add_space(if *level <= 2 { 26.0 } else { 18.0 } * zoom);
            let mut style = Style::new(heading_size(*level) * zoom);
            style.strong = true;
            let text: String = spans.iter().map(Span::plain).collect();
            let drawn = ui.vertical(|ui| show_spans(ui, spans, state, style));
            if let Some(section) = *target
                && localobsidian_md::outline::section_matches(section, &text)
            {
                ui.scroll_to_rect(drawn.response.rect, Some(egui::Align::TOP));
                *target = None;
            }
            ui.add_space(6.0 * zoom);
            drawn.inner
        }
        Block::Paragraph(spans) => {
            let hit = show_spans(ui, spans, state, Style::new(BODY_SIZE * zoom));
            ui.add_space(14.0 * zoom);
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
                        if let Some(found) = show_span(ui, span, state, Style::new(BODY_SIZE * zoom)) {
                            hit = Some(found);
                        }
                    }
                });
                // A list is a stack of lines, not a paragraph: 5 px between items keeps the rhythm.
                ui.add_space(5.0 * zoom);
            }
            ui.add_space(8.0);
            hit
        }
        Block::Code { language, text } => {
            ui.add_space(4.0);
            if !language.is_empty() {
                ui.label(RichText::new(language).small().weak());
            }
            egui::Frame::new().fill(ui.visuals().code_bg_color).inner_margin(12.0).corner_radius(6.0).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new(text).monospace().size(13.0 * zoom));
            });
            // A fence is a block: two of them in a row keep the rhythm of any other pair of blocks
            // (they used to sit a few pixels apart, which read as one broken block).
            ui.add_space(16.0);
            None
        }
        Block::Query { source } => {
            let outcome = state.query_vault(source);
            let mut hit = None;
            egui::Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .stroke(egui::Stroke::new(1.0, ui.visuals().weak_text_color().gamma_multiply(0.35)))
                .corner_radius(6)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tl!("query")).small().weak());
                        ui.label(RichText::new(crate::i18n::trn(outcome.total as u64, "{n} result", "{n} results")).small().strong());
                    });
                    // A line the parser cannot use is said out loud: a query that quietly matches
                    // everything would be worse than an error.
                    for mistake in &outcome.mistakes {
                        ui.label(
                            RichText::new(crate::i18n::tr_fmt("not understood: {line}", &[("line", mistake)]))
                                .small()
                                .color(ui.visuals().warn_fg_color),
                        );
                    }
                    if outcome.total == 0 && outcome.mistakes.is_empty() {
                        ui.label(RichText::new(tl!("no note matches this query")).small().weak());
                    }
                    for note in &outcome.notes {
                        let label = ui.add(egui::Label::new(Style::new(BODY_SIZE * zoom).rich(note.name.clone())).sense(egui::Sense::click()));
                        if label.on_hover_text(tl!("Open this note")).clicked() {
                            hit = Some(Hit::Link(note.name.clone()));
                        }
                    }
                    if outcome.total > outcome.notes.len() {
                        let more = outcome.total - outcome.notes.len();
                        ui.label(RichText::new(crate::i18n::trn(more as u64, "+{n} more", "+{n} more")).small().weak());
                    }
                });
            ui.add_space(12.0);
            hit
        }
        Block::Quote(inner) => {
            let weak = ui.visuals().weak_text_color();
            let quoted = ui.indent(ui.scope_id().with("quote"), |ui| {
                ui.visuals_mut().override_text_color = Some(weak);
                show_blocks(ui, inner, state, images, target, zoom)
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
        Block::Callout { kind, title, body } => show_callout(ui, kind, title, body, state, images, target, zoom),
        Block::Table { headers, rows, aligns } => show_table(ui, headers, rows, aligns, state, zoom),
        Block::Image { destination, alt } => {
            show_image(ui, destination, alt, state, images);
            None
        }
        Block::Rule => {
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
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
    zoom: f32,
) -> Option<Hit> {
    let color = callout_color(kind);
    let mut hit = None;
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.55)))
        .inner_margin(14.0)
        .corner_radius(8.0)
        .show(ui, |ui| {
            // A callout is a block: it takes the width of the note, not the width of its title.
            ui.set_min_width(ui.available_width());
            if !title.trim().is_empty() {
                ui.label(RichText::new(title).strong().size(15.0 * zoom).color(color));
                ui.add_space(4.0);
            }
            if let Some(found) = show_blocks(ui, body, state, images, target, zoom) {
                hit = Some(found);
            }
        });
    ui.add_space(12.0);
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
/// Room above and below it: 12 + a 15 px line is the ~40 px row the Electron app has.
const TABLE_PAD_Y: f32 = 12.0;
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
fn show_table(ui: &mut egui::Ui, headers: &[Vec<Span>], rows: &[Vec<Vec<Span>>], aligns: &[Align], state: &VaultState, zoom: f32) -> Option<Hit> {
    let columns = headers.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return None;
    }
    let body = Style::new(14.0 * zoom);
    let head = Style::new(14.0 * zoom).strong();
    let available = ui.available_width().max(TABLE_MIN_COL * columns as f32);

    // What each column would like: its widest cell, measured without wrapping.
    let mut natural = vec![TABLE_MIN_COL; columns];
    // And the narrowest it may get without breaking a word in the middle: the widest single word it
    // holds, header included. Five columns at 800 px do not fit "Noradrenalina"; the honest answer is
    // a table wider than the page with a horizontal scroll, not a word cut in half.
    let mut floors = vec![TABLE_MIN_COL; columns];
    for (column, cell) in headers.iter().enumerate() {
        natural[column] = natural[column].max(text_width(ui, cell, head) + 2.0 * TABLE_PAD);
        floors[column] = floors[column].max(word_width(ui, cell, head) + 2.0 * TABLE_PAD);
    }
    for row in rows {
        for (column, cell) in row.iter().enumerate() {
            if let Some(slot) = natural.get_mut(column) {
                *slot = (*slot).max(text_width(ui, cell, body) + 2.0 * TABLE_PAD);
            }
            if let Some(slot) = floors.get_mut(column) {
                *slot = (*slot).max(word_width(ui, cell, body) + 2.0 * TABLE_PAD);
            }
        }
    }
    let widths = column_widths(available, &natural, &floors);

    // Each row's height, from the tallest cell wrapped at the width it will get.
    let heights: Vec<f32> = std::iter::once(headers)
        .chain(rows.iter().map(|row| row.as_slice()))
        .enumerate()
        .map(|(index, cells)| {
            let style = if index == 0 { head } else { body };
            row_height(ui, cells, &widths, style, TABLE_PAD, TABLE_PAD_Y).min(TABLE_MAX_ROW)
        })
        .collect();
    let total_width: f32 = widths.iter().sum();
    if total_width > available + 1.0 {
        // Wider than the pane: it scrolls sideways rather than folding the words.
        let mut hit = None;
        egui::ScrollArea::horizontal().id_salt("table-scroll").show(ui, |ui| {
            hit = paint_table(ui, headers, rows, aligns, &widths, &heights, state, head, body);
        });
        ui.add_space(6.0);
        return hit;
    }
    let hit = paint_table(ui, headers, rows, aligns, &widths, &heights, state, head, body);
    ui.add_space(6.0);
    hit
}

/// Draws the measured table at the cursor. Split out because a table wider than the pane is drawn
/// inside a horizontal scroll area instead of on the page.
fn paint_table(
    ui: &mut egui::Ui,
    headers: &[Vec<Span>],
    rows: &[Vec<Vec<Span>>],
    aligns: &[Align],
    widths: &[f32],
    heights: &[f32],
    state: &VaultState,
    head: Style,
    body: Style,
) -> Option<Hit> {
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
    for width in widths {
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
                egui::pos2(x + TABLE_PAD, y + TABLE_PAD_Y),
                egui::vec2((width - 2.0 * TABLE_PAD).max(8.0), (height - 2.0 * TABLE_PAD_Y).max(8.0)),
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
    hit
}

/// The width of a cell's text as one line.
fn text_width(ui: &egui::Ui, spans: &[Span], style: Style) -> f32 {
    let text: String = spans.iter().map(Span::plain).collect();
    let font = egui::FontId::proportional(style.size);
    ui.painter().layout_no_wrap(text, font, egui::Color32::WHITE).size().x
}

/// The height a cell needs when wrapped at `width`, plus the padding above and below.
fn row_height(ui: &egui::Ui, cells: &[Vec<Span>], widths: &[f32], style: Style, pad_x: f32, pad_y: f32) -> f32 {
    let font = egui::FontId::proportional(style.size);
    let mut tallest: f32 = ui.text_style_height(&egui::TextStyle::Body);
    for (column, cell) in cells.iter().enumerate() {
        let width = widths.get(column).copied().unwrap_or(TABLE_MIN_COL).max(8.0) - 2.0 * pad_x;
        let text: String = cell.iter().map(Span::plain).collect();
        let height = ui.painter().layout(text, font.clone(), egui::Color32::WHITE, width).size().y;
        tallest = tallest.max(height);
    }
    tallest + 2.0 * pad_y
}

/// The width of the widest single word of a cell: no column is narrower than this, so a line never
/// breaks inside a word (a table that will not fit then scrolls sideways instead).
fn word_width(ui: &egui::Ui, spans: &[Span], style: Style) -> f32 {
    let text: String = spans.iter().map(Span::plain).collect();
    let font = egui::FontId::proportional(style.size);
    text.split_whitespace()
        .map(|word| ui.painter().layout_no_wrap(word.to_string(), font.clone(), egui::Color32::WHITE).size().x)
        .fold(0.0_f32, f32::max)
}

/// How wide each column is: what it asked for, grown to fill the page when there is room, shrunk but
/// never below the width of its longest word when there is not.
fn column_widths(available: f32, natural: &[f32], floors: &[f32]) -> Vec<f32> {
    let asked: f32 = natural.iter().sum();
    if asked <= available && asked > 0.0 {
        let slack = available - asked;
        return natural.iter().map(|width| width + slack * (width / asked)).collect();
    }
    let mut widths = natural.to_vec();
    // Shrink proportionally in a few passes: one pass overshoots, because taking the same share from
    // a column that already hit its floor would make the others too narrow.
    for _ in 0..6 {
        let total: f32 = widths.iter().sum();
        if total <= available {
            break;
        }
        let scale = (available / total).min(1.0);
        for (index, width) in widths.iter_mut().enumerate() {
            let floor = floors.get(index).copied().unwrap_or(TABLE_MIN_COL).max(TABLE_MIN_COL);
            *width = (*width * scale).max(floor);
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
        1 => 29.0,
        2 => 21.0,
        3 => 18.0,
        4 => 16.0,
        5 => 15.0,
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
            // With the find bar looking for something, a text run is drawn in pieces so the matches
            // can be lit - and the one the reader is on scrolls into view.
            if !crate::find::is_searching(&state.find) {
                ui.label(style.rich(drawable(ui, text, style.size)));
                return None;
            }
            let needle = state.find.query.trim().to_string();
            let ranges = crate::find::ranges(text, &needle);
            if ranges.is_empty() {
                ui.label(style.rich(drawable(ui, text, style.size)));
                return None;
            }
            let accent = crate::theme::accent(ui.visuals().dark_mode);
            let mut at = 0;
            for (start, end) in ranges {
                if start > at {
                    ui.label(style.rich(drawable(ui, &text[at..start], style.size)));
                }
                let index = state.find.seen.get();
                let current = index == state.find.current;
                let fill = accent.gamma_multiply(if current { 0.55 } else { 0.22 });
                let piece = ui.label(style.rich(drawable(ui, &text[start..end], style.size)).background_color(fill));
                if current {
                    ui.scroll_to_rect(piece.rect, Some(egui::Align::Center));
                }
                state.find.seen.set(index + 1);
                at = end;
            }
            if at < text.len() {
                ui.label(style.rich(drawable(ui, &text[at..], style.size)));
            }
            None
        }
        Span::Code(text) => {
            // A code chip is a chip, not a warning: the same violet-grey family as the tag pills,
            // not the olive the theme's code background lands on.
            let tint = crate::theme::accent(ui.visuals().dark_mode).gamma_multiply(if ui.visuals().dark_mode { 0.22 } else { 0.12 });
            ui.label(style.rich(text.clone()).monospace().background_color(tint));
            None
        }
        Span::Highlight(text) => {
            // `==text==`: a tint of the accent, the same one the export uses.
            let accent = crate::theme::accent(ui.visuals().dark_mode);
            ui.label(style.rich(drawable(ui, text, style.size)).background_color(accent.gamma_multiply(0.22)));
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
/// The app ships no emoji font (a decision, not an oversight). An emoji in a note is therefore never
/// drawn - a monochrome pictogram among coloured text reads as a mistake, and a hollow box reads
/// worse - while the symbol ranges (arrows, maths signs) are drawn when the shipped fonts have them.
/// The note keeps every character, and the editor shows it as written; only the reading view is
/// quieter.
fn drawable(ui: &egui::Ui, text: &str, size: f32) -> String {
    if !text.chars().any(|c| is_emoji(c) || is_symbol(c)) {
        return text.to_string();
    }
    let font = egui::FontId::proportional(size);
    ui.ctx().fonts_mut(|fonts| {
        text.chars()
            .filter(|c| {
                if is_emoji(*c) {
                    false
                } else if is_symbol(*c) {
                    fonts.has_glyph(&font, *c)
                } else {
                    true
                }
            })
            .collect()
    })
}

/// The emoji planes, and the invisible characters that glue an emoji together.
pub(crate) fn is_emoji(c: char) -> bool {
    matches!(u32::from(c), 0x20E3 | 0x200D | 0xFE0F | 0x1F000..=0x1FAFF)
}

/// The symbol ranges a note uses as text: arrows, maths operators and the common signs.
pub(crate) fn is_symbol(c: char) -> bool {
    matches!(u32::from(c), 0x2190..=0x21FF | 0x2200..=0x22FF | 0x2300..=0x23FF | 0x2600..=0x27BF | 0x2B00..=0x2BFF)
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
        // A link that resolves is the accent; one that points at a note nobody wrote is readable
        // text with an underline - grey on a coloured callout was close to invisible.
        let (color, underline) = if resolves { (ui.visuals().hyperlink_color, false) } else { (ui.visuals().text_color().gamma_multiply(0.9), true) };
        let mut rich = style.colored(color).rich(text);
        if underline {
            rich = rich.underline();
        }
        let response = ui.add(egui::Label::new(rich).sense(egui::Sense::click()));
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
