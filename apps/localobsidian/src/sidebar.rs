//! The sidebar: the filter box, the tag chips, the vault's tree and the backlinks of the open note.
//!
//! The tree is the vault as read from disk, ordered files-first (see `localobsidian_vault::sorted`);
//! the filter narrows it while you type and never reads note text - a filter on a tree has to stay
//! instant, and full-text search is a different feature (Ctrl+Shift+F).

use std::path::PathBuf;

use localobsidian_vault as vault;

use crate::panels;
use crate::tl;
use crate::vault_state::VaultState;

/// What the user asked the sidebar for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarAction {
    None,
    OpenNote(PathBuf),
}

/// Draws the sidebar. Returns the note the user clicked, if any.
pub fn show(ui: &mut egui::Ui, state: &mut VaultState) -> SidebarAction {
    let mut action = SidebarAction::None;

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut state.filter).hint_text(tl!("Filter notes")).desired_width(f32::INFINITY - 60.0));
        if !state.filter.is_empty() && ui.small_button("x").on_hover_text(tl!("Clear the filter")).clicked() {
            state.filter.clear();
        }
    });
    panels::tags(ui, state);
    if !state.filter.trim().is_empty() || state.tag_filter.is_some() {
        let kept = state.filtered_count();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(crate::i18n::tr_fmt("{count} notes", &[("count", &kept.to_string())])).small().weak());
            if let Some(tag) = state.tag_filter.clone()
                && ui.small_button(format!("#{tag} x")).on_hover_text(tl!("Clear the tag")).clicked()
            {
                state.tag_filter = None;
            }
        });
    }
    ui.separator();

    // The tree scrolls; the backlinks stay pinned under it.
    egui::Panel::bottom("backlinks").resizable(true).default_size(120.0).show(ui, |ui| {
        if let Some(path) = panels::backlinks(ui, state) {
            action = SidebarAction::OpenNote(path);
        }
    });

    state.rebuild_filtered_tree();
    let tree = state.filtered_tree();
    let empty = tree.is_empty();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if empty {
            ui.label(egui::RichText::new(tl!("no note matches the filter")).weak());
            return;
        }
        match show_nodes(ui, tree, state) {
            SidebarAction::None => {}
            other => action = other,
        }
    });
    action
}

fn show_nodes(ui: &mut egui::Ui, nodes: &[vault::Node], state: &VaultState) -> SidebarAction {
    let mut action = SidebarAction::None;
    for node in nodes {
        match node.kind {
            vault::Kind::Directory => {
                egui::CollapsingHeader::new(egui::RichText::new(&node.name)).id_salt(&node.path).default_open(true).show(ui, |ui| {
                    let inner = show_nodes(ui, &node.children, state);
                    if inner != SidebarAction::None {
                        action = inner;
                    }
                });
            }
            vault::Kind::File => {
                let selected = state.active.as_deref() == Some(node.path.as_path());
                if ui.selectable_label(selected, egui::RichText::new(&node.name)).clicked() {
                    action = SidebarAction::OpenNote(node.path.clone());
                }
            }
        }
    }
    action
}
