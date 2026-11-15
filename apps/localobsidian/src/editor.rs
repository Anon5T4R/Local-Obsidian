//! The note pane: the editor, and the reading view it toggles into.
//!
//! The editor is plain text on purpose - the file is Markdown, and a note is edited where it is
//! read. The reading view draws the block model from `localobsidian_md::preview`; a `[[wikilink]]`
//! there is a real link: clicking it asks the app to open the note it names.

use egui::{RichText, TextStyle};

use localobsidian_md::links;
use localobsidian_md::preview::{Block, Span};

use crate::tl;
use crate::vault_state::VaultState;

/// What the user asked the note pane for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorAction {
    None,
    Save,
    /// Open the note a wikilink names.
    Navigate(String),
}

/// How a run of spans is drawn: the block's size, plus what the inline markers added.
#[derive(Clone, Copy)]
struct Style {
    size: f32,
    strong: bool,
    italics: bool,
    strike: bool,
    /// `None` = the theme's text colour.
    color: Option<egui::Color32>,
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

    fn colored(self, color: egui::Color32) -> Self {
        Self { color: Some(color), ..self }
    }
}

/// Draws the note pane.
pub fn show(ui: &mut egui::Ui, state: &mut VaultState) -> EditorAction {
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

    if state.preview_open {
        state.refresh_preview();
        let blocks = state.preview().to_vec();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if let Some(target) = show_blocks(ui, &blocks, state) {
                action = EditorAction::Navigate(target);
            }
        });
    } else {
        let mut text = state.buffer.clone();
        let output = egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(&mut text).font(TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(24).code_editor())
        });
        if output.inner.changed() {
            state.edit(text);
        }
    }
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
            let label = if state.preview_open { tl!("Edit") } else { tl!("Read") };
            if ui.selectable_label(false, label).on_hover_text(tl!("Switch between the editor and the reading view")).clicked() {
                state.preview_open = !state.preview_open;
            }
        });
    });
}

/// Draws blocks, returning the wikilink target the user clicked.
fn show_blocks(ui: &mut egui::Ui, blocks: &[Block], state: &VaultState) -> Option<String> {
    let mut clicked = None;
    for block in blocks {
        match block {
            Block::Heading { level, spans } => {
                ui.add_space(6.0);
                let mut style = Style::new(heading_size(*level));
                style.strong = true;
                if let Some(target) = show_spans(ui, spans, state, style) {
                    clicked = Some(target);
                }
                ui.add_space(2.0);
            }
            Block::Paragraph(spans) => {
                if let Some(target) = show_spans(ui, spans, state, Style::new(14.5)) {
                    clicked = Some(target);
                }
                ui.add_space(4.0);
            }
            Block::List { ordered, items } => {
                for (index, item) in items.iter().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        let marker = if *ordered { format!("{}.", index + 1) } else { "•".to_string() };
                        ui.label(RichText::new(marker).weak());
                        for span in item {
                            if let Some(target) = show_span(ui, span, state, Style::new(14.5)) {
                                clicked = Some(target);
                            }
                        }
                    });
                }
                ui.add_space(4.0);
            }
            Block::Code { language, text } => {
                if !language.is_empty() {
                    ui.label(RichText::new(language).small().weak());
                }
                egui::Frame::new().fill(ui.visuals().code_bg_color).inner_margin(8.0).corner_radius(4.0).show(ui, |ui| {
                    ui.label(RichText::new(text).monospace());
                });
                ui.add_space(4.0);
            }
            Block::Quote(inner) => {
                let weak = ui.visuals().weak_text_color();
                let quoted = ui.indent(ui.scope_id().with("quote"), |ui| {
                    ui.visuals_mut().override_text_color = Some(weak);
                    show_blocks(ui, inner, state)
                });
                if let Some(target) = quoted.inner {
                    clicked = Some(target);
                }
                // The bar on the left is what makes a quote read as one, as in the source file.
                let rect = quoted.response.rect;
                if rect.height() > 0.0 {
                    let bar = egui::Rect::from_min_size(egui::pos2(rect.left() - 10.0, rect.top()), egui::vec2(3.0, rect.height()));
                    ui.painter().rect_filled(bar, 1.0, weak);
                }
            }
            Block::Rule => {
                ui.separator();
            }
        }
    }
    clicked
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
fn show_spans(ui: &mut egui::Ui, spans: &[Span], state: &VaultState, style: Style) -> Option<String> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        for span in spans {
            if let Some(target) = show_span(ui, span, state, style) {
                clicked = Some(target);
            }
        }
    });
    clicked
}

fn show_span(ui: &mut egui::Ui, span: &Span, state: &VaultState, style: Style) -> Option<String> {
    match span {
        Span::Text(text) => {
            ui.label(style.rich(text.clone()));
            None
        }
        Span::Code(text) => {
            ui.label(style.rich(text.clone()).monospace().background_color(ui.visuals().code_bg_color));
            None
        }
        Span::Strong(inner) => show_spans_inline(ui, inner, state, style.strong()),
        Span::Emphasis(inner) => show_spans_inline(ui, inner, state, style.italic()),
        Span::Strike(inner) => show_spans_inline(ui, inner, state, style.struck()),
        Span::Link { spans, destination } => show_link(ui, spans, destination, state, style),
    }
}

/// A nested run (bold inside italic), drawn into the run already open.
fn show_spans_inline(ui: &mut egui::Ui, spans: &[Span], state: &VaultState, style: Style) -> Option<String> {
    let mut clicked = None;
    for span in spans {
        if let Some(target) = show_span(ui, span, state, style) {
            clicked = Some(target);
        }
    }
    clicked
}

fn show_link(ui: &mut egui::Ui, spans: &[Span], destination: &str, state: &VaultState, style: Style) -> Option<String> {
    let text: String = spans.iter().map(Span::plain).collect();
    if let Some(target) = links::from_url(destination) {
        let resolves = state.resolve(&target).is_some();
        let color = if resolves { ui.visuals().hyperlink_color } else { ui.visuals().weak_text_color() };
        let response = ui.add(egui::Label::new(style.colored(color).rich(text)).sense(egui::Sense::click()));
        if resolves && response.clicked() {
            return Some(target);
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
