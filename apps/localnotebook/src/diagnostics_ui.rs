//! The vault diagnostics: what the reader cannot see while reading one note at a time.
//!
//! Three findings, each a row that opens the note it is about: links naming a note nobody wrote,
//! notes nothing reaches, and names that are not unique (a `[[link]]` to one of them lands on
//! whichever the resolver picks).

use crate::i18n;
use crate::tl;
use crate::vault_state::VaultState;

/// Draws the panel. Returns the note the reader asked to open.
pub fn show(ui: &mut egui::Ui, state: &VaultState) -> Option<std::path::PathBuf> {
    let diagnostics = state.diagnostics();
    let mut open: Option<std::path::PathBuf> = None;

    if diagnostics.is_clean() {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(tl!("nothing to fix: every link lands and every note is reachable")).weak());
        return None;
    }

    let summary = [
        i18n::trn(diagnostics.broken_links.len() as u64, "{n} broken link", "{n} broken links"),
        i18n::trn(diagnostics.orphans.len() as u64, "{n} orphan note", "{n} orphan notes"),
        i18n::trn(diagnostics.duplicates.len() as u64, "{n} duplicated name", "{n} duplicated names"),
    ]
    .join(" · ");
    ui.label(egui::RichText::new(summary).small().weak());
    ui.separator();

    egui::ScrollArea::vertical().id_salt("diagnostics").auto_shrink([false, false]).show(ui, |ui| {
        section(ui, i18n::trn(diagnostics.broken_links.len() as u64, "{n} broken link", "{n} broken links"));
        for link in &diagnostics.broken_links {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(&link.target).strong());
                ui.label(egui::RichText::new("—").weak());
                for source in &link.from {
                    if let Some(note) = state.files.iter().find(|note| note.name.eq_ignore_ascii_case(source)) {
                        if ui.add(egui::Label::new(egui::RichText::new(source).small()).sense(egui::Sense::click())).clicked() {
                            open = Some(note.path.clone());
                        }
                    } else {
                        ui.label(egui::RichText::new(source).small().weak());
                    }
                }
            });
        }

        section(ui, i18n::trn(diagnostics.orphans.len() as u64, "{n} orphan note", "{n} orphan notes"));
        for relative in &diagnostics.orphans {
            if ui.add(egui::Label::new(egui::RichText::new(relative).small()).sense(egui::Sense::click())).clicked()
                && let Some(note) = state.files.iter().find(|note| &note.relative_path == relative)
            {
                open = Some(note.path.clone());
            }
        }

        section(ui, i18n::trn(diagnostics.duplicates.len() as u64, "{n} duplicated name", "{n} duplicated names"));
        for duplicate in &diagnostics.duplicates {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(&duplicate.name).strong());
                ui.label(egui::RichText::new("—").weak());
                for relative in &duplicate.files {
                    if ui.add(egui::Label::new(egui::RichText::new(relative).small()).sense(egui::Sense::click())).clicked()
                        && let Some(note) = state.files.iter().find(|note| &note.relative_path == relative)
                    {
                        open = Some(note.path.clone());
                    }
                }
            });
        }
    });
    open
}

/// A heading for one kind of finding.
fn section(ui: &mut egui::Ui, title: String) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(title).strong());
    ui.add_space(2.0);
}
