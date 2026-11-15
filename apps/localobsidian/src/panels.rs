//! The panels that answer "where is it?": search, outline and backlinks.
//!
//! Each of them is read-only over [`VaultState`] and hands back what the user asked to open, so the
//! window decides where the click lands. The search panel is the port of the upstream
//! `SearchPanel`: a query box with the `tag:`/`path:`/`file:`/`"phrase"`/`-excluded` grammar, a
//! `.*` switch, results grouped by note (the most relevant first) and a few lines each - never
//! hiding occurrences without saying how many are hidden.

use std::path::PathBuf;

use crate::i18n;
use crate::tl;
use crate::vault_state::{SearchHit, VaultState};

/// How many matching lines of a note the panel shows before "and N more".
const VISIBLE_LINES: usize = 5;

/// What a click in a panel asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelAction {
    None,
    /// Open a note, optionally putting the caret on a line.
    Open {
        path: PathBuf,
        line: Option<usize>,
    },
    /// Open the note the active one is in, at a section.
    GoToSection(String),
}

/// The search panel.
pub fn search(ui: &mut egui::Ui, state: &mut VaultState) -> PanelAction {
    let mut action = PanelAction::None;

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let response =
            ui.add(egui::TextEdit::singleline(&mut state.search.query).hint_text(tl!("Search the vault")).desired_width(f32::INFINITY - 30.0));
        response.request_focus();
        // Enter searches at once instead of waiting for the typing to settle.
        if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            state.run_search_now();
        }
        let regex = state.search.regex;
        if ui.selectable_label(regex, ".*").on_hover_text(tl!("Match the text as a regular expression")).clicked() {
            state.search.regex = !regex;
            // A switch means a new search, not a wait for the next keystroke.
            state.search.invalidate();
        }
    });
    // The hint is the grammar itself: the operators are literal, so it is not translated.
    ui.label(egui::RichText::new(r#"tag:x  path:x  file:x  -word  "exact phrase""#).small().weak());

    if state.search.invalid {
        ui.label(egui::RichText::new(tl!("that pattern is not valid")).color(ui.visuals().warn_fg_color));
        return action;
    }
    if state.search.results.is_empty() {
        if !state.search.query.trim().is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tl!("no note matches")).weak());
        }
        return action;
    }

    let notes = state.search.results.len();
    let lines: usize = state.search.results.iter().map(|hit| hit.matches.len()).sum();
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(i18n::tr_fmt("{notes} notes, {lines} matching lines", &[("notes", &notes.to_string()), ("lines", &lines.to_string())]))
            .small()
            .weak(),
    );
    ui.separator();

    egui::ScrollArea::vertical().id_salt("search-results").auto_shrink([false, false]).show(ui, |ui| {
        let hits = state.search.results.clone();
        for hit in &hits {
            if let Some(requested) = show_hit(ui, hit, state) {
                action = requested;
            }
        }
    });
    action
}

/// One note of the results: its name, then its matching lines.
fn show_hit(ui: &mut egui::Ui, hit: &SearchHit, state: &mut VaultState) -> Option<PanelAction> {
    let mut action = None;
    let expanded = state.search.expanded.contains(&hit.path);
    ui.horizontal(|ui| {
        if ui.selectable_label(false, egui::RichText::new(&hit.name).strong()).clicked() {
            action = Some(PanelAction::Open { path: hit.path.clone(), line: None });
        }
        ui.label(egui::RichText::new(hit.matches.len().to_string()).small().weak());
        ui.label(egui::RichText::new(&hit.relative_path).small().weak());
    });
    let shown = if expanded { hit.matches.len() } else { hit.matches.len().min(VISIBLE_LINES) };
    for m in hit.matches.iter().take(shown) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(m.line.to_string()).small().weak());
            let line = egui::RichText::new(&m.text).monospace().size(12.5);
            if ui.add(egui::Label::new(line).sense(egui::Sense::click())).clicked() {
                action = Some(PanelAction::Open { path: hit.path.clone(), line: Some(m.line) });
            }
        });
    }
    if hit.matches.len() > VISIBLE_LINES {
        let hidden = hit.matches.len() - shown;
        let label =
            if expanded { tl!("show fewer lines").to_string() } else { i18n::tr_fmt("and {count} more lines", &[("count", &hidden.to_string())]) };
        if ui.small_button(label).clicked() {
            if expanded {
                state.search.expanded.remove(&hit.path);
            } else {
                state.search.expanded.insert(hit.path.clone());
            }
        }
    }
    ui.add_space(4.0);
    action
}

/// The outline of the open note.
pub fn outline(ui: &mut egui::Ui, state: &mut VaultState) -> PanelAction {
    let mut action = PanelAction::None;
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Outline")).strong());
    ui.separator();

    state.refresh_outline();
    let headings = state.outline().to_vec();
    if headings.is_empty() {
        ui.label(egui::RichText::new(tl!("this note has no headings")).weak());
        return action;
    }
    egui::ScrollArea::vertical().id_salt("outline").auto_shrink([false, false]).show(ui, |ui| {
        for heading in &headings {
            let indent = f32::from(heading.level.saturating_sub(1)) * 10.0;
            ui.horizontal(|ui| {
                ui.add_space(indent);
                let label = if heading.level == 1 { egui::RichText::new(&heading.text).strong() } else { egui::RichText::new(&heading.text) };
                if ui.add(egui::Label::new(label).sense(egui::Sense::click())).clicked() {
                    action = PanelAction::GoToSection(heading.text.clone());
                }
            });
        }
    });
    action
}

/// The backlinks of the open note, for the bottom of the sidebar.
pub fn backlinks(ui: &mut egui::Ui, state: &VaultState) -> Option<PathBuf> {
    let mut opened = None;
    let links = state.backlinks_of_active();
    let title = match links.len() {
        0 => tl!("Backlinks").to_string(),
        count => i18n::tr_fmt("Backlinks ({count})", &[("count", &count.to_string())]),
    };
    egui::CollapsingHeader::new(egui::RichText::new(title)).id_salt("backlinks").default_open(true).show(ui, |ui| {
        if state.active.is_none() {
            ui.label(egui::RichText::new(tl!("No note is open")).weak());
            return;
        }
        if links.is_empty() {
            ui.label(egui::RichText::new(tl!("no note links this one")).weak());
            return;
        }
        for name in links {
            if ui.selectable_label(false, name.clone()).clicked()
                && let Some(note) = state.resolve(&name)
            {
                opened = Some(note.path.clone());
            }
        }
    });
    opened
}

/// The tag chips, for the top of the sidebar: most used first, one click filters the tree.
pub fn tags(ui: &mut egui::Ui, state: &mut VaultState) {
    let tags = state.tag_counts();
    if tags.is_empty() {
        return;
    }
    let title = i18n::tr_fmt("Tags ({count})", &[("count", &tags.len().to_string())]);
    egui::CollapsingHeader::new(egui::RichText::new(title)).id_salt("tags").default_open(true).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (tag, count) in &tags {
                let active = state.tag_filter.as_deref() == Some(tag.as_str());
                let label = format!("{tag} {count}");
                if ui.selectable_label(active, egui::RichText::new(label).small()).clicked() {
                    state.tag_filter = if active { None } else { Some(tag.clone()) };
                }
            }
        });
    });
}
