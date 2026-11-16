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
    /// Open a document (PDF, DOCX, ODT, EPUB, TXT) with the desktop's own handler.
    OpenBinary(PathBuf),
    /// Show a folder in the file manager.
    RevealFolder(PathBuf),
    /// Make AI flashcards from this note (a new note beside it).
    Flashcards(PathBuf),
}

/// Draws the sidebar. Returns the note the user clicked, if any.
///
/// One panel, three blocks, each with the height it was given: the tree takes what is left after the
/// tag section and the backlinks claimed theirs. Nested side panels were the bug behind the tree and
/// the backlinks drawing in the same rectangle - a panel inside a `Ui` reserves space for itself but
/// not for what comes after it, so every block started again at the top.
pub fn show(ui: &mut egui::Ui, state: &mut VaultState) -> SidebarAction {
    let mut action = SidebarAction::None;

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        // The row minus the clear button, never the window: a field that wants infinity widens the
        // panel that holds it.
        let room = (ui.available_width() - 60.0).max(80.0);
        ui.add(egui::TextEdit::singleline(&mut state.filter).hint_text(tl!("Filter notes")).desired_width(room));
        if !state.filter.is_empty() && ui.small_button("x").on_hover_text(tl!("Clear the filter")).clicked() {
            state.filter.clear();
        }
    });
    if !state.filter.trim().is_empty() || state.tag_filter.is_some() {
        let kept = state.filtered_count();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(crate::i18n::trn(kept as u64, "{n} note", "{n} notes")).small().weak());
            if let Some(tag) = state.tag_filter.clone()
                && ui.small_button(format!("#{tag} x")).on_hover_text(tl!("Clear the tag")).clicked()
            {
                state.tag_filter = None;
            }
        });
    }
    ui.separator();

    // The three blocks, sized from what the panel actually has. The backlinks take whatever is left
    // after the tree and the tags: a share computed up front was 10 px off and clipped the last row.
    let total = ui.available_height();
    let content_bottom = ui.max_rect().bottom();
    let tags_height = if state.tags_open { (total * 0.28).clamp(96.0, 220.0) } else { HEADER_HEIGHT };
    let tree_height = (total - tags_height - 200.0).max(120.0);

    state.rebuild_filtered_tree();
    let tree = state.filtered_tree();
    let empty = tree.is_empty();
    egui::ScrollArea::vertical().id_salt("tree-scroll").max_height(tree_height).auto_shrink([false, false]).show(ui, |ui| {
        if empty {
            ui.label(egui::RichText::new(tl!("no note matches the filter")).weak());
            return;
        }
        match show_nodes(ui, tree, state) {
            SidebarAction::None => {}
            other => action = other,
        }
    });

    if panels::tags(ui, state, tags_height) {
        state.tags_open = !state.tags_open;
    }
    ui.separator();
    // Whatever is left *of the panel* is the backlinks', up to a point: measured from the bottom of
    // the sidebar, not from `available_height` - the tree shrinks to its content, and the difference
    // is space that is not really there.
    let backlinks_height = (content_bottom - ui.cursor().top() - 8.0).clamp(60.0, 180.0);
    let backlinks = ui.allocate_ui(egui::vec2(ui.available_width(), backlinks_height), |ui| panels::backlinks(ui, state, backlinks_height));
    if let Some(path) = backlinks.inner {
        action = SidebarAction::OpenNote(path);
    }
    action
}

/// The row a folded section occupies: its header and nothing else.
const HEADER_HEIGHT: f32 = 24.0;

fn show_nodes(ui: &mut egui::Ui, nodes: &[vault::Node], state: &VaultState) -> SidebarAction {
    let mut action = SidebarAction::None;
    for node in nodes {
        match node.kind {
            vault::Kind::Directory => {
                let shown = egui::CollapsingHeader::new(egui::RichText::new(&node.name)).id_salt(&node.path).default_open(true).show(ui, |ui| {
                    let inner = show_nodes(ui, &node.children, state);
                    if inner != SidebarAction::None {
                        action = inner;
                    }
                });
                // A folder right-click belongs to the folder: this is where the vault is a folder.
                let shown = shown.header_response.context_menu(|ui| {
                    if ui.button(tl!("Open the folder in the file manager")).clicked() {
                        action = SidebarAction::RevealFolder(node.path.clone());
                        ui.close();
                    }
                });
                let _ = shown;
            }
            vault::Kind::File => {
                let selected = state.active.as_deref() == Some(node.path.as_path());
                let row = ui.selectable_label(selected, egui::RichText::new(&node.name));
                if row.clicked() {
                    action = SidebarAction::OpenNote(node.path.clone());
                }
                // The note's own menu: what can be done to *this* note, where the note is.
                row.context_menu(|ui| {
                    if ui.button(tl!("Open")).clicked() {
                        action = SidebarAction::OpenNote(node.path.clone());
                        ui.close();
                    }
                    if ui.button(tl!("Create flashcards from this note")).clicked() {
                        action = SidebarAction::Flashcards(node.path.clone());
                        ui.close();
                    }
                });
            }
            vault::Kind::Binary => {
                // A document the app does not edit: listed, muted, and opened by the desktop.
                let row = ui.add(egui::Label::new(egui::RichText::new(&node.name).weak()).sense(egui::Sense::click()));
                let row = row.on_hover_text(tl!("Open with the system"));
                if row.clicked() {
                    action = SidebarAction::OpenBinary(node.path.clone());
                }
                let row = row.context_menu(|ui| {
                    if ui.button(tl!("Open with the system")).clicked() {
                        action = SidebarAction::OpenBinary(node.path.clone());
                        ui.close();
                    }
                    if ui.button(tl!("Show in the file manager")).clicked() {
                        action = SidebarAction::RevealFolder(node.path.clone());
                        ui.close();
                    }
                });
                let _ = row;
            }
        }
    }
    action
}
