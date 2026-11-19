//! The command palette (Ctrl+K or the search icon of the top bar): every menu command, by the same name, run through
//! the same code the menu runs. The box, the filter and the keys are the suite's `craft-palette`.

use craft_icons::ThemeMode;
use craft_palette::Command;
use egui::{Key, Modifiers};

use super::{Deck, LocalApp, ViewMode, keys};
use crate::i18n;
use crate::tl;

/// The sort orders of View > Sort notes, in the menu's order.
const SORTS: [(localobsidian_vault::Sort, &str, &str); 3] = [
    (localobsidian_vault::Sort::Name, "name", "Name"),
    (localobsidian_vault::Sort::NameDesc, "name_desc", "Name (Z to A)"),
    (localobsidian_vault::Sort::Modified, "modified", "Last modified"),
];

/// The views of View > Note view, in the menu's order.
const VIEWS: [(ViewMode, &str, &str); 3] = [(ViewMode::Edit, "edit", "Edit"), (ViewMode::Read, "read", "Read"), (ViewMode::Split, "split", "Split")];

impl LocalApp {
    /// What the palette lists, in menu order. A command that needs a vault or a note is listed (so the user learns it
    /// exists) and greyed, as in the menu.
    pub(super) fn commands(&self, ctx: &egui::Context) -> Vec<Command> {
        let (file, view, review, ai, help) = (tl!("File"), tl!("View"), tl!("Review"), tl!("AI"), tl!("Help"));
        let vault = self.vault.root.is_some();
        let note = self.vault.active.is_some();
        let free = !self.pickers.busy();
        let command = Modifiers::COMMAND;
        let mut list = vec![
            Command::new("file.open", tl!("Open a vault…"), file).enabled(free),
            Command::new("file.new", tl!("New note…"), file).enabled(vault),
            Command::new("file.import_document", tl!("Import a Word or OpenDocument file…"), file).enabled(vault && free),
            Command::new("file.import_anki", tl!("Import Anki cards (text)…"), file).enabled(vault && free),
            Command::new("file.rename", tl!("Rename note…"), file).enabled(note),
            Command::new("file.save", tl!("Save"), file).shortcut(keys(ctx, command, Key::S)).enabled(note && self.vault.dirty),
            Command::new("file.export_html", tl!("Export the note as HTML…"), file).enabled(note && free),
            Command::new("file.export_pdf", tl!("Export the note as PDF…"), file).enabled(note && free),
            Command::new("file.settings", tl!("Settings…"), file),
            Command::new("file.exit", tl!("Exit"), file),
            Command::new("view.sidebar", tl!("Show the sidebar"), view),
        ];
        for (_, id, label) in VIEWS {
            let label = i18n::tr_fmt("Note view: {view}", &[("view", tl!(label))]);
            list.push(Command::new(format!("view.mode.{id}"), label, view));
        }
        for mode in ThemeMode::EVERY {
            let label = i18n::tr_fmt("Theme: {mode}", &[("mode", tl!(mode.label()))]);
            list.push(Command::new(format!("view.theme.{}", mode.id()), label, view));
        }
        list.extend([
            Command::new("view.zoom_in", tl!("Zoom in"), view),
            Command::new("view.zoom_out", tl!("Zoom out"), view),
            Command::new("view.zoom_reset", tl!("Reset the zoom"), view).shortcut(keys(ctx, command, Key::Num0)),
        ]);
        for (_, id, label) in SORTS {
            let label = i18n::tr_fmt("Sort notes: {order}", &[("order", tl!(label))]);
            list.push(Command::new(format!("view.sort.{id}"), label, view));
        }
        list.extend([
            Command::new("view.search", tl!("Search the vault"), view).shortcut(keys(ctx, command | Modifiers::SHIFT, Key::F)).enabled(vault),
            Command::new("view.outline", tl!("Show the outline"), view).shortcut(keys(ctx, command | Modifiers::SHIFT, Key::O)),
            Command::new("view.connections", tl!("Show the connections"), view),
            Command::new("view.diagnostics", tl!("Vault diagnostics"), view),
            Command::new("view.random", tl!("Random note"), view).shortcut(keys(ctx, command, Key::R)).enabled(vault),
            Command::new("view.calendar", tl!("Show the calendar"), view),
            Command::new("view.refresh", tl!("Refresh the vault"), view).enabled(vault),
            Command::new("review.vault", tl!("Review the whole vault"), review).enabled(vault),
            Command::new("review.note", tl!("Review this note"), review).enabled(note),
        ]);
        if let Some(tag) = &self.vault.tag_filter {
            let label = i18n::tr_fmt("Review #{tag}", &[("tag", tag)]);
            list.push(Command::new("review.tag", label, review));
        }
        list.extend([
            Command::new("review.stats", tl!("Review statistics"), review),
            Command::new("review.close", tl!("Close the review panel"), review).enabled(self.review.open),
            Command::new("ai.chat", tl!("Chat about the note"), ai),
            Command::new("ai.connection", tl!("Connection…"), ai),
            Command::new("ai.flashcards", tl!("Create flashcards from this note"), ai)
                .shortcut(keys(ctx, command | Modifiers::SHIFT, Key::K))
                .enabled(note),
            Command::new("help.keys", tl!("Keyboard shortcuts and help"), help),
            Command::new("help.updates", tl!("Check for Updates…"), help),
            Command::new("help.about", tl!("About Local Obsidian"), help),
        ]);
        list
    }

    /// Runs what the palette returned, through the same calls as the menus.
    pub(super) fn run_command(&mut self, ctx: &egui::Context, id: &str) {
        match id {
            "file.open" => self.ask_vault(ctx),
            "file.new" => self.begin_new_note(),
            "file.import_document" => self.ask_document(ctx),
            "file.import_anki" => self.ask_anki(ctx),
            "file.rename" => self.begin_rename(),
            "file.save" => self.save(),
            "file.export_html" => self.ask_export(ctx),
            "file.export_pdf" => self.ask_export_pdf(ctx),
            "file.settings" => self.settings_open = true,
            "file.exit" => {
                self.remember();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            "view.sidebar" => {
                self.sidebar_open = !self.sidebar_open;
                self.remember();
            }
            "view.zoom_in" => self.step_zoom(1),
            "view.zoom_out" => self.step_zoom(-1),
            "view.zoom_reset" => {
                self.vault.zoom = 1.0;
                self.remember();
            }
            "view.search" => self.toggle_search(),
            "view.outline" => {
                self.vault.outline_open = !self.vault.outline_open;
                self.remember();
            }
            "view.connections" => self.set_connections(!self.connections_open),
            "view.diagnostics" => self.diagnostics_open = !self.diagnostics_open,
            "view.random" => self.open_random_note(),
            "view.calendar" => self.calendar_open = !self.calendar_open,
            "view.refresh" => self.reload(),
            "review.vault" => self.review.start(&self.vault, Deck::All),
            "review.note" => self.review_note(),
            "review.tag" => {
                if let Some(tag) = self.vault.tag_filter.clone() {
                    self.review.start(&self.vault, Deck::Tag(tag));
                }
            }
            "review.stats" => self.review.stats_open = true,
            "review.close" => self.review.open = false,
            "ai.chat" => self.chat.open = !self.chat.open,
            "ai.connection" => self.chat.settings_open = true,
            "ai.flashcards" => self.start_flashcards(),
            "help.keys" => self.help_open = true,
            "help.updates" => {
                // The outcome shows in the About window, like the menu's check.
                self.about_open = true;
                self.updater.check_now(ctx);
            }
            "help.about" => self.about_open = true,
            _ => {}
        }
        if let Some(mode) = id.strip_prefix("view.theme.").map(ThemeMode::from_id) {
            self.set_mode(ctx, mode);
        }
        if let Some(name) = id.strip_prefix("view.mode.")
            && let Some((mode, _, _)) = VIEWS.into_iter().find(|(_, key, _)| *key == name)
        {
            self.vault.view = mode;
            self.remember();
        }
        if let Some(name) = id.strip_prefix("view.sort.")
            && let Some((sort, _, _)) = SORTS.into_iter().find(|(_, key, _)| *key == name)
        {
            self.vault.set_sort(sort);
            self.remember();
        }
    }
}
