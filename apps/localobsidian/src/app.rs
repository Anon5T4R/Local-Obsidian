//! The window: title bar (when the app draws it), menus, the vault page, the About window that
//! fronts the update check, and the small dialogs the vault needs (new note, rename).
//!
//! The page is the sidebar plus the note pane; the vault's own state lives in
//! [`crate::vault_state::VaultState`], and the views hand back what the user asked for.

use std::path::PathBuf;

use craft_update::updater::{Action, Status, Updater};

use crate::calendar;
use crate::editor::{self, EditorAction};
use crate::graph;
use crate::i18n;
use crate::images::Images;
use crate::panels::{self, PanelAction};
use crate::pickers::{Pick, Pickers};
use crate::prefs::Prefs;
use crate::review_ui::{Deck, Review};
use crate::sidebar::{self, SidebarAction};
use crate::suite_entry::APP;
use crate::templates;
use crate::tl;
use crate::vault_state::{VaultState, ViewMode};

/// A small dialog the page opens on top of itself.
enum Dialog {
    NewNote { name: String, template: String },
    Rename { name: String },
}

pub struct LocalApp {
    updater: Updater,
    pickers: Pickers,
    /// Who draws the window frame (probed before the window was created).
    frame: craft_window::Frame,
    about_open: bool,
    /// Whether the shortcuts window is on screen.
    help_open: bool,
    /// The `language` setting: [`i18n::AUTO`] or a code from [`i18n::LANGUAGES`].
    language: String,
    prefs: Prefs,
    vault: VaultState,
    /// The pictures the reading view has decoded (`![[foto.png]]`).
    images: Images,
    /// The review session and its statistics panel.
    review: Review,
    /// Whether the link graph is on screen.
    graph_open: bool,
    /// Whether the calendar popover is on screen, and the month it shows.
    calendar_open: bool,
    calendar_month: String,
    /// The vault folder as the OS sees it changing.
    watcher: Option<crate::watcher::Watcher>,
    /// When the folder was last seen changing (the debounce clock).
    fs_change: Option<std::time::Instant>,
    /// The note this app wrote last, and when: its own save is not news.
    saved_at: Option<(std::path::PathBuf, std::time::Instant)>,
    sidebar_open: bool,
    dialog: Option<Dialog>,
    /// The last vault is opened on the first frame, not in the constructor: building the window
    /// must not wait for a scan of a large folder.
    restore_pending: bool,
}

impl LocalApp {
    pub fn new(cc: &eframe::CreationContext<'_>, frame: craft_window::Frame) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        let prefs = Prefs::load();
        i18n::set_language(&prefs.language);

        let mut vault = VaultState::default();
        vault.sort = prefs.sort;
        vault.view = prefs.view;
        vault.outline_open = prefs.outline_open;

        Self {
            updater: Updater::new(&APP, env!("CARGO_PKG_VERSION")),
            pickers: Pickers::default(),
            frame,
            about_open: false,
            help_open: false,
            language: prefs.language.clone(),
            restore_pending: prefs.last_vault.is_some(),
            sidebar_open: prefs.sidebar_open,
            prefs,
            vault,
            images: Images::default(),
            review: Review::default(),
            graph_open: false,
            calendar_open: false,
            calendar_month: crate::calendar::today_iso(),
            watcher: None,
            fs_change: None,
            saved_at: None,
            dialog: None,
        }
    }

    fn remember(&self) {
        let mut prefs = self.prefs.clone();
        prefs.sort = self.vault.sort;
        prefs.view = self.vault.view;
        prefs.outline_open = self.vault.outline_open;
        prefs.sidebar_open = self.sidebar_open;
        prefs.language = self.language.clone();
        prefs.save();
    }

    /// The menu bar: the app's commands and Help.
    fn menus(&mut self, ui: &mut egui::Ui) {
        let mut quit = false;
        let has_note = self.vault.active.is_some();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tl!("File"), |ui| {
                if ui.add_enabled(!self.pickers.busy(), egui::Button::new(tl!("Open a vault..."))).clicked() {
                    self.ask_vault(ui.ctx());
                    ui.close();
                }
                if ui.add_enabled(self.vault.root.is_some(), egui::Button::new(tl!("New note..."))).clicked() {
                    self.dialog = Some(Dialog::NewNote { name: String::new(), template: String::new() });
                    ui.close();
                }
                if ui
                    .add_enabled(self.vault.root.is_some() && !self.pickers.busy(), egui::Button::new(tl!("Import a Word or OpenDocument file...")))
                    .on_hover_text(tl!("Reads a .docx or .odt into a note. Pictures are left out."))
                    .clicked()
                {
                    self.ask_document(ui.ctx());
                    ui.close();
                }
                if ui
                    .add_enabled(self.vault.root.is_some() && !self.pickers.busy(), egui::Button::new(tl!("Import Anki cards (text)...")))
                    .on_hover_text(tl!("A text export from Anki (.txt, .tsv, .csv). A .apkg package is not read."))
                    .clicked()
                {
                    self.ask_anki(ui.ctx());
                    ui.close();
                }
                if ui.add_enabled(has_note, egui::Button::new(tl!("Rename note..."))).clicked() {
                    let name = self
                        .vault
                        .active
                        .as_ref()
                        .and_then(|path| path.file_stem())
                        .map(|stem| stem.to_string_lossy().to_string())
                        .unwrap_or_default();
                    self.dialog = Some(Dialog::Rename { name });
                    ui.close();
                }
                ui.separator();
                if ui.add_enabled(has_note && self.vault.dirty, egui::Button::new(tl!("Save"))).clicked() {
                    self.save();
                    ui.close();
                }
                if ui.add_enabled(has_note && !self.pickers.busy(), egui::Button::new(tl!("Export the note as HTML..."))).clicked() {
                    self.ask_export(ui.ctx());
                    ui.close();
                }
                ui.separator();
                if ui.button(tl!("Quit Local Obsidian")).clicked() {
                    quit = true;
                    ui.close();
                }
            });
            ui.menu_button(tl!("View"), |ui| {
                if ui.checkbox(&mut self.sidebar_open, tl!("Show the sidebar")).clicked() {
                    self.remember();
                    ui.close();
                }
                ui.menu_button(tl!("Note view"), |ui| {
                    for (mode, label) in [(ViewMode::Edit, tl!("Edit")), (ViewMode::Read, tl!("Read")), (ViewMode::Split, tl!("Split"))] {
                        if ui.radio(self.vault.view == mode, label).clicked() {
                            self.vault.view = mode;
                            self.remember();
                            ui.close();
                        }
                    }
                });
                ui.menu_button(tl!("Sort notes"), |ui| {
                    for (sort, label) in [
                        (localobsidian_vault::Sort::Name, tl!("Name")),
                        (localobsidian_vault::Sort::NameDesc, tl!("Name (Z to A)")),
                        (localobsidian_vault::Sort::Modified, tl!("Last modified")),
                    ] {
                        if ui.radio(self.vault.sort == sort, label).clicked() {
                            self.vault.set_sort(sort);
                            self.remember();
                            ui.close();
                        }
                    }
                });
                ui.separator();
                if ui.add_enabled(self.vault.root.is_some(), egui::Button::new(tl!("Search the vault"))).clicked() {
                    self.toggle_search();
                    ui.close();
                }
                if ui.checkbox(&mut self.vault.outline_open, tl!("Show the outline")).clicked() {
                    self.remember();
                    ui.close();
                }
                if ui.checkbox(&mut self.graph_open, tl!("Show the graph")).clicked() {
                    ui.close();
                }
                if ui.checkbox(&mut self.calendar_open, tl!("Show the calendar")).clicked() {
                    ui.close();
                }
                ui.separator();
                if ui.add_enabled(self.vault.root.is_some(), egui::Button::new(tl!("Refresh the vault"))).clicked() {
                    self.reload();
                    ui.close();
                }
            });
            ui.menu_button(tl!("Review"), |ui| {
                if ui.add_enabled(self.vault.root.is_some(), egui::Button::new(tl!("Review the whole vault"))).clicked() {
                    self.review.start(&self.vault, Deck::All);
                    ui.close();
                }
                let note = self.vault.active.as_ref().and_then(|path| {
                    self.vault.files.iter().find(|file| &file.path == path).map(|file| (file.relative_path.clone(), file.name.clone()))
                });
                if ui.add_enabled(note.is_some(), egui::Button::new(tl!("Review this note"))).clicked()
                    && let Some((relative, _)) = note
                {
                    self.review.start(&self.vault, Deck::Note(relative));
                    ui.close();
                }
                if let Some(tag) = self.vault.tag_filter.clone()
                    && ui.button(i18n::tr_fmt("Review #{tag}", &[("tag", &tag)])).clicked()
                {
                    self.review.start(&self.vault, Deck::Tag(tag));
                    ui.close();
                }
                ui.separator();
                if ui.button(tl!("Review statistics")).clicked() {
                    self.review.stats_open = true;
                    ui.close();
                }
                if self.review.open && ui.button(tl!("Close the review panel")).clicked() {
                    self.review.open = false;
                    ui.close();
                }
            });
            ui.menu_button(tl!("Help"), |ui| {
                if ui.button(tl!("Keyboard shortcuts and help")).clicked() {
                    self.help_open = true;
                    ui.close();
                }
                ui.separator();
                if ui.button(tl!("Check for Updates.")).clicked() {
                    // The outcome shows in the About window, which is where a check belongs.
                    self.about_open = true;
                    self.updater.check_now(ui.ctx());
                    ui.close();
                }
                if ui.button(tl!("About Local Obsidian")).clicked() {
                    self.about_open = true;
                    ui.close();
                }
            });
        });
        if quit {
            self.remember();
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn page(&mut self, ui: &mut egui::Ui) {
        if self.vault.root.is_none() {
            self.welcome(ui);
            return;
        }
        if self.sidebar_open {
            egui::Panel::left("sidebar").resizable(true).default_size(240.0).show(ui, |ui| match sidebar::show(ui, &mut self.vault) {
                SidebarAction::None => {}
                SidebarAction::OpenNote(path) => self.vault.open_note(&path),
            });
        }
        if self.vault.search.open {
            egui::Panel::right("search").resizable(true).default_size(320.0).show(ui, |ui| match panels::search(ui, &mut self.vault) {
                PanelAction::None => {}
                PanelAction::Open { path, line } => self.vault.show_result(&path, line),
                PanelAction::GoToSection(section) => self.goto_section(&section),
            });
        }
        if self.vault.outline_open {
            egui::Panel::right("outline").resizable(true).default_size(220.0).show(ui, |ui| match panels::outline(ui, &mut self.vault) {
                PanelAction::None => {}
                PanelAction::Open { path, line } => self.vault.show_result(&path, line),
                PanelAction::GoToSection(section) => self.goto_section(&section),
            });
        }
        if self.review.open {
            egui::Panel::right("review").resizable(true).default_size(320.0).show(ui, |ui| {
                self.review.ui(ui, &mut self.vault);
            });
        }
        if self.review.stats_open {
            egui::Panel::right("review-stats").resizable(true).default_size(300.0).show(ui, |ui| {
                self.review.stats_ui(ui, &mut self.vault);
            });
        }
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let notes = self.vault.files.len();
                ui.label(egui::RichText::new(i18n::tr_fmt("{count} notes", &[("count", &notes.to_string())])).small().weak());
                let due = self.vault.srs_stats().due;
                if due > 0 {
                    ui.separator();
                    if ui.small_button(i18n::tr_fmt("{count} cards due", &[("count", &due.to_string())])).clicked() {
                        self.review.start(&self.vault, Deck::All);
                    }
                }
                if let Some(message) = &self.vault.message {
                    ui.separator();
                    ui.label(egui::RichText::new(message).small());
                }
            });
        });
        egui::CentralPanel::default_margins().show(ui, |ui| match editor::show(ui, &mut self.vault, &mut self.images) {
            EditorAction::None => {}
            EditorAction::Save => self.save(),
            EditorAction::Navigate(target) => self.navigate(&target),
            EditorAction::ToggleTask(index) => {
                self.vault.toggle_task(index);
            }
        });
    }

    /// What a vault-less window shows: the one action worth taking, and what a vault is.
    fn welcome(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(48.0);
            ui.heading(tl!("Local Obsidian"));
            ui.add_space(4.0);
            ui.label(tl!("A vault is a folder of Markdown notes: no database, no lock-in."));
            ui.add_space(12.0);
            if ui.add_enabled(!self.pickers.busy(), egui::Button::new(tl!("Open a vault..."))).clicked() {
                self.ask_vault(ui.ctx());
            }
            if let Some(message) = &self.vault.message {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(message).small().weak());
            }
        });
    }

    fn dialogs(&mut self, ui: &mut egui::Ui) {
        let Some(dialog) = self.dialog.take() else { return };
        let mut submitted = false;
        let mut cancel = false;
        let mut open = true;
        match dialog {
            Dialog::NewNote { mut name, mut template } => {
                if template.is_empty() {
                    template = "blank".to_string();
                }
                // The picker offers the built-ins and the user's own notes under `_templates/`.
                let user_templates: Vec<(String, String)> = self
                    .vault
                    .files
                    .iter()
                    .filter(|note| templates::is_template_path(&note.relative_path))
                    .map(|note| (note.path.to_string_lossy().to_string(), note.name.clone()))
                    .collect();
                egui::Window::new(tl!("New note"))
                    .open(&mut open)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .show(ui.ctx(), |ui| {
                        ui.label(tl!("Note name"));
                        let response = ui.add(egui::TextEdit::singleline(&mut name).desired_width(300.0));
                        response.request_focus();
                        submitted |= response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(tl!("Template")).small().weak());
                        for builtin in templates::builtins() {
                            let label = tl!(builtin.label);
                            if ui.selectable_label(template == builtin.id, label).clicked() {
                                template = builtin.id.to_string();
                            }
                        }
                        for (path, label) in &user_templates {
                            if ui.selectable_label(template == *path, label).clicked() {
                                template = path.clone();
                            }
                        }
                        if !user_templates.is_empty() {
                            ui.label(
                                egui::RichText::new(i18n::tr_fmt("your own templates live in {folder}/", &[("folder", templates::TEMPLATES_DIR)]))
                                    .small()
                                    .weak(),
                            );
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            submitted |= ui.add_enabled(!name.trim().is_empty(), egui::Button::new(tl!("Create"))).clicked();
                            cancel |= ui.button(tl!("Cancel")).clicked();
                        });
                    });
                if submitted && !name.trim().is_empty() {
                    let now = chrono::Local::now();
                    // A user template is a note of the vault: read it and expand its placeholders.
                    let content = match self.vault.files.iter().find(|note| note.path.to_string_lossy() == template) {
                        Some(note) => match std::fs::read_to_string(&note.path) {
                            Ok(raw) => templates::expand_vars(&raw, name.trim(), now),
                            Err(error) => {
                                self.vault.message = Some(format!("{}: {error}", tl!("could not read the template")));
                                String::new()
                            }
                        },
                        None => templates::expand_builtin(&template, name.trim(), now),
                    };
                    if !content.is_empty() {
                        match self.vault.create_note_with(name.trim(), &content, None) {
                            Ok(path) => {
                                // The file we just wrote is not an outside change.
                                self.saved_at = Some((path, std::time::Instant::now()));
                                self.remember();
                            }
                            Err(error) => self.vault.message = Some(format!("{}: {error}", tl!("could not create the note"))),
                        }
                    }
                } else if open && !cancel {
                    self.dialog = Some(Dialog::NewNote { name, template });
                }
            }
            Dialog::Rename { mut name } => {
                egui::Window::new(tl!("Rename note"))
                    .open(&mut open)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .show(ui.ctx(), |ui| {
                        ui.label(tl!("Note name"));
                        let response = ui.add(egui::TextEdit::singleline(&mut name).desired_width(240.0));
                        response.request_focus();
                        submitted |= response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        ui.horizontal(|ui| {
                            submitted |= ui.add_enabled(!name.trim().is_empty(), egui::Button::new(tl!("Rename"))).clicked();
                            cancel |= ui.button(tl!("Cancel")).clicked();
                        });
                    });
                if submitted && !name.trim().is_empty() {
                    match self.vault.rename_active(name.trim()) {
                        Ok(count) => self.vault.message = Some(i18n::tr_fmt("{count} links updated", &[("count", &count.to_string())])),
                        Err(error) => self.vault.message = Some(format!("{}: {error}", tl!("could not rename the note"))),
                    }
                } else if open && !cancel {
                    self.dialog = Some(Dialog::Rename { name });
                }
            }
        }
    }

    /// The link graph, in its own window.
    fn graph(&mut self, ui: &mut egui::Ui) {
        if !self.graph_open {
            return;
        }
        let mut open = true;
        egui::Window::new(tl!("Graph")).open(&mut open).default_size([720.0, 620.0]).show(ui.ctx(), |ui| {
            if let Some(path) = graph::show(ui, &self.vault) {
                self.vault.open_note(&path);
            }
        });
        self.graph_open = open;
    }

    /// The calendar window: a day with a note opens it, a day without creates it.
    fn calendar(&mut self, ui: &mut egui::Ui) {
        if !self.calendar_open {
            return;
        }
        let mut open = true;
        egui::Window::new(tl!("Calendar")).open(&mut open).default_size([330.0, 340.0]).resizable(false).show(ui.ctx(), |ui| {
            let marked = |iso: &str| self.vault.files.iter().any(|note| note.name == iso);
            if let Some(iso) = calendar::show(ui, &mut self.calendar_month, &marked, i18n::code()) {
                self.open_daily_note(&iso);
                self.calendar_open = false;
            }
        });
        self.calendar_open = open;
    }

    /// Asks for a `.docx`/`.odt` to read into a note.
    fn ask_document(&mut self, ctx: &egui::Context) {
        let start = self.prefs.last_vault.as_ref().map(|path| path.display().to_string());
        if !self.pickers.ask_document(ctx, tl!("Import a document"), start.as_deref()) {
            log::debug!("a picker is already open");
        }
    }

    /// Asks for an Anki text export.
    fn ask_anki(&mut self, ctx: &egui::Context) {
        let start = self.prefs.last_vault.as_ref().map(|path| path.display().to_string());
        if !self.pickers.ask_anki(ctx, tl!("Import an Anki deck"), start.as_deref()) {
            log::debug!("a picker is already open");
        }
    }

    /// Reads the document at `path` into a note next to the open one.
    ///
    /// The upstream converts in the source's own folder; a document picked from outside the vault
    /// has no folder here, so the note lands beside the open one (or in the vault's root) under the
    /// document's name. A note that already has that name is opened instead of overwritten.
    fn import_document(&mut self, path: &std::path::Path) {
        let name = path.file_stem().map(|stem| stem.to_string_lossy().trim().to_string()).unwrap_or_default();
        if name.is_empty() {
            return;
        }
        if let Some(existing) = self.vault.files.iter().find(|note| note.name.eq_ignore_ascii_case(&name)) {
            let existing = existing.path.clone();
            self.vault.open_note(&existing);
            self.vault.message = Some(i18n::tr_fmt("the note {name} already exists; it was opened", &[("name", &name)]));
            return;
        }
        let markdown = match localobsidian_convert::to_markdown(path) {
            Ok(markdown) => markdown,
            Err(error) => {
                self.vault.message = Some(format!("{}: {error}", tl!("could not read the document")));
                return;
            }
        };
        if markdown.trim().is_empty() {
            self.vault.message = Some(tl!("the document has no text to import").to_string());
            return;
        }
        match self.vault.create_note_with(&name, &markdown, None) {
            Ok(created) => {
                // Our own write is not an outside change.
                self.saved_at = Some((created, std::time::Instant::now()));
                self.vault.message = Some(i18n::tr_fmt("imported {name}", &[("name", &name)]));
            }
            Err(error) => self.vault.message = Some(format!("{}: {error}", tl!("could not create the note"))),
        }
    }

    /// Reads the Anki text export at `path` into notes of card callouts, the way the upstream's
    /// `srs:anki-write` does: a deck of 2000 cards becomes notes of 100, so opening one is cheap.
    ///
    /// A text export never carries media, so nothing has to be copied - the cards that show a picture
    /// keep the reference and the note says how many did.
    fn import_anki(&mut self, path: &std::path::Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                self.vault.message = Some(format!("{}: {error}", tl!("could not read the export")));
                return;
            }
        };
        let import = localobsidian_review::from_anki_text(&text);
        if import.cards.is_empty() {
            self.vault.message = Some(tl!("no cards were found in the export").to_string());
            return;
        }
        let deck = path.file_stem().map(|stem| stem.to_string_lossy().trim().to_string()).unwrap_or_default();
        let deck = if deck.is_empty() { tl!("Anki deck").to_string() } else { deck };
        let chunks = localobsidian_review::chunk_cards(&import.cards, CARDS_PER_NOTE);
        let root = self.vault.root.clone();
        let mut written = 0;
        for (index, chunk) in chunks.iter().enumerate() {
            let base = if chunks.len() == 1 { deck.clone() } else { format!("{deck} {}", index + 1) };
            let name = self.free_note_name(&base);
            let markdown = localobsidian_review::anki_to_markdown(&name, chunk);
            match self.vault.create_note_with(&name, &markdown, root.as_deref()) {
                Ok(created) => {
                    self.saved_at = Some((created, std::time::Instant::now()));
                    written += 1;
                }
                Err(error) => {
                    self.vault.message = Some(format!("{}: {error}", tl!("could not create the note")));
                    return;
                }
            }
        }
        let mut message =
            i18n::tr_fmt("imported {cards} cards into {notes} notes", &[("cards", &import.cards.len().to_string()), ("notes", &written.to_string())]);
        if import.with_media > 0 {
            message.push_str(&format!(" — {}", tl!("media was left out")));
        }
        self.vault.message = Some(message);
    }

    /// `base`, or `base 2`, `base 3`… - the first name no note in the vault has.
    fn free_note_name(&self, base: &str) -> String {
        let taken = |name: &str| self.vault.files.iter().any(|note| note.name.eq_ignore_ascii_case(name));
        if !taken(base) {
            return base.to_string();
        }
        for number in 2..1000 {
            let candidate = format!("{base} {number}");
            if !taken(&candidate) {
                return candidate;
            }
        }
        base.to_string()
    }

    /// Asks where to write the open note's HTML.
    fn ask_export(&mut self, ctx: &egui::Context) {
        let Some(active) = self.vault.active.clone() else { return };
        let name = active.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
        let name = if name.is_empty() { tl!("note").to_string() } else { name };
        if !self.pickers.ask_save(ctx, tl!("Export as HTML"), &format!("{name}.html")) {
            log::debug!("a picker is already open");
        }
    }

    /// Writes the open note as one HTML page, and copies the pictures it shows beside it.
    ///
    /// The page is what the reading view draws: the same blocks, the same callouts, the stylesheet
    /// inlined and the palette following the reader's system. A `[[Nota]]` becomes `Nota.html`, so
    /// exporting the notes it points at into the same folder makes the link work.
    fn export_html(&mut self, path: std::path::PathBuf) {
        let Some(active) = self.vault.active.clone() else {
            self.vault.message = Some(tl!("open a note first").to_string());
            return;
        };
        let name = active.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
        self.vault.refresh_preview();
        let html = localobsidian_md::html::document(&name, i18n::code(), self.vault.preview());
        if let Err(error) = std::fs::write(&path, html) {
            self.vault.message = Some(format!("{}: {error}", tl!("could not write the page")));
            return;
        }

        // The pictures keep the vault's `_attachments/` layout, so the note's own paths stay right.
        let mut copied = 0;
        if let Some(folder) = path.parent() {
            let wanted = localobsidian_md::html::images(self.vault.preview());
            for destination in wanted {
                let Some(source) = self.vault.attachment_path(&destination) else { continue };
                let Some(file_name) = source.file_name().map(|name| name.to_os_string()) else { continue };
                let target_folder = folder.join("_attachments");
                if std::fs::create_dir_all(&target_folder).is_err() {
                    continue;
                }
                if std::fs::copy(&source, target_folder.join(file_name)).is_ok() {
                    copied += 1;
                }
            }
        }

        let file = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
        let mut message = i18n::tr_fmt("exported to {file}", &[("file", &file)]);
        if copied > 0 {
            message.push_str(" · ");
            message.push_str(&i18n::tr_fmt("{count} images copied", &[("count", &copied.to_string())]));
        }
        self.vault.message = Some(message);
    }

    /// Opens the daily note for `iso`, creating it from `_templates/daily.md` when the user has one.
    fn open_daily_note(&mut self, iso: &str) {
        if !calendar::is_daily_note_name(iso) {
            return;
        }
        if let Some(note) = self.vault.files.iter().find(|note| note.name == iso) {
            let path = note.path.clone();
            self.vault.open_note(&path);
            return;
        }
        let template = self
            .vault
            .files
            .iter()
            .find(|note| templates::is_template_path(&note.relative_path) && note.name.eq_ignore_ascii_case("daily"))
            .map(|note| note.path.clone());
        let now = calendar::local_midnight(iso).unwrap_or_else(chrono::Local::now);
        let content = match template.and_then(|path| std::fs::read_to_string(path).ok()) {
            Some(raw) => templates::expand_vars(&raw, iso, now),
            None => format!("# {iso}\n\n"),
        };
        let root = self.vault.root.clone();
        match self.vault.create_note_with(iso, &content, root.as_deref()) {
            Ok(path) => {
                self.saved_at = Some((path, std::time::Instant::now()));
                self.remember();
            }
            Err(error) => self.vault.message = Some(format!("{}: {error}", tl!("could not create the note"))),
        }
    }

    /// The help window: what the app does, the keys it answers, and the syntax a note holds.
    fn help(&mut self, ui: &mut egui::Ui) {
        if !self.help_open {
            return;
        }
        let mut open = true;
        egui::Window::new(tl!("Help")).open(&mut open).default_size([560.0, 540.0]).show(ui.ctx(), |ui| {
            egui::ScrollArea::vertical().id_salt("help-body").auto_shrink([false, false]).show(ui, help_body);
        });
        self.help_open = open;
    }

    fn about(&mut self, ui: &mut egui::Ui) {
        let mut open = self.about_open;
        egui::Window::new(tl!("About Local Obsidian"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                ui.heading(APP.name);
                ui.label(i18n::tr_fmt("version {version}", &[("version", env!("CARGO_PKG_VERSION"))]));
                ui.label(APP.tagline);
                ui.add_space(6.0);
                ui.hyperlink_to(APP.repo, APP.repo_url());
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let checking = matches!(self.updater.status(), Status::Checking);
                    if ui.add_enabled(!checking, egui::Button::new(tl!("Check for Updates."))).clicked() {
                        self.updater.check_now(ui.ctx());
                    }
                    match self.updater.status() {
                        Status::Idle => {}
                        Status::Checking => {
                            ui.spinner();
                        }
                        Status::UpToDate => {
                            ui.label(tl!("You are on the newest version."));
                        }
                        Status::Failed(reason) => {
                            ui.label(i18n::tr_fmt("checking for updates failed: {reason}", &[("reason", reason)]));
                        }
                    };
                });
                ui.add_space(8.0);
                self.language_picker(ui);
            });
        self.about_open = open;
    }

    /// The interface language, straight from [`i18n::LANGUAGES`].
    fn language_picker(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(tl!("Language"));
            let current = i18n::code();
            egui::ComboBox::from_id_salt("language").selected_text(i18n::name_of(current)).show_ui(ui, |ui| {
                for (code, name) in i18n::languages() {
                    if ui.selectable_label(code == current, name).clicked() && code != current {
                        self.language = code.to_string();
                        i18n::set_lang(i18n::resolve(&self.language));
                        self.remember();
                    }
                }
            });
            if self.language == i18n::AUTO {
                ui.label(tl!("(following the system)"));
            }
        });
    }

    /// The app-drawn title bar (GNOME on Wayland; see `craft-window`).
    fn header_bar(&mut self, ui: &mut egui::Ui, layout: &craft_window::Layout) {
        egui::Panel::top("title_bar").show(ui, |ui| {
            craft_window::title_bar::empty_area(ui, ui.max_rect());
            ui.horizontal(|ui| {
                for button in &layout.left {
                    window_button(ui, *button);
                }
                self.menus(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for button in layout.right.iter().rev() {
                        window_button(ui, *button);
                    }
                });
            });
        });
    }

    fn ask_vault(&mut self, ctx: &egui::Context) {
        let start = self.prefs.last_vault.as_ref().map(|path| path.display().to_string());
        if !self.pickers.ask_folder(ctx, tl!("Open a vault..."), start.as_deref()) {
            log::debug!("a folder picker is already open");
        }
    }

    fn open_vault(&mut self, path: PathBuf) {
        match self.vault.open(path.clone()) {
            Ok(()) => {
                self.vault.open_first_note();
                // The OS watch: a note changed elsewhere (another editor, a sync client) shows up.
                self.watcher = crate::watcher::Watcher::start(&path);
                self.fs_change = None;
                self.prefs.last_vault = Some(path);
                self.remember();
            }
            Err(error) => self.vault.message = Some(format!("{}: {error}", tl!("could not open the vault"))),
        }
    }

    fn reload(&mut self) {
        match self.vault.reload() {
            Ok(()) => self.vault.message = Some(tl!("vault refreshed").to_string()),
            Err(error) => self.vault.message = Some(format!("{}: {error}", tl!("could not read the vault"))),
        }
    }

    fn save(&mut self) {
        match self.vault.save() {
            Ok(()) => {
                self.vault.message = Some(tl!("saved").to_string());
                // Our own write is not news from the outside: the watcher must ignore it.
                if let Some(path) = self.vault.active.clone() {
                    self.saved_at = Some((path, std::time::Instant::now()));
                }
            }
            Err(error) => self.vault.message = Some(format!("{}: {error}", tl!("could not save the note"))),
        }
    }

    /// What to do about a change another program made in the vault.
    ///
    /// The upstream rule, kept: **the open note wins** - if it is dirty, the text the user is typing
    /// is not replaced, and the window says so instead. Everything else is re-read.
    fn react_to_disk(&mut self, changed: &[std::path::PathBuf]) {
        // A folder can appear or vanish without its notes being reported one by one.
        let relevant: Vec<std::path::PathBuf> =
            changed.iter().filter(|path| crate::watcher::is_note_path(path) || path.extension().is_none()).cloned().collect();
        if relevant.is_empty() {
            return;
        }
        let changed = relevant.as_slice();
        let now = std::time::Instant::now();
        let own = self.saved_at.as_ref().filter(|(_, at)| now.duration_since(*at) < std::time::Duration::from_secs(3)).map(|(path, _)| path.clone());
        if changed.iter().all(|path| Some(path) == own.as_ref()) {
            return;
        }
        let touches_open = changed.iter().any(|path| Some(path) == self.vault.active.as_ref());
        if touches_open && self.vault.dirty {
            self.vault.message = Some(tl!("the note changed on disk; your unsaved text was kept").to_string());
            return;
        }
        match self.vault.reload() {
            Ok(()) => {
                if touches_open {
                    self.vault.reload_open_note();
                }
                self.vault.message = Some(tl!("the vault changed on disk and was reloaded").to_string());
            }
            Err(error) => self.vault.message = Some(format!("{}: {error}", tl!("could not read the vault"))),
        }
    }

    /// Follows a `[[wikilink]]`: opens the note it names, at the section when it names one.
    fn navigate(&mut self, target: &str) {
        let parsed = localobsidian_md::parse_target(target);
        if let Some(note) = self.vault.resolve(parsed.note) {
            let path = note.path.clone();
            self.vault.open_note(&path);
            if let Some(section) = parsed.hash {
                self.goto_section(section);
            }
        } else {
            self.vault.message = Some(i18n::tr_fmt("no note is called {name}", &[("name", parsed.note)]));
        }
    }

    /// Shows the reading view scrolled to `section` of the open note.
    fn goto_section(&mut self, section: &str) {
        self.vault.pending_section = Some(section.to_string());
        self.vault.view = ViewMode::Read;
        self.remember();
    }

    /// Opens or closes the search panel. Opening it puts the caret in the box.
    fn toggle_search(&mut self) {
        self.vault.search.open = !self.vault.search.open;
        if !self.vault.search.open {
            self.vault.search.query.clear();
            self.vault.search.results.clear();
            self.vault.search.invalidate();
        }
    }
}

/// Cards per note when an Anki deck is imported: a note of 2000 cards takes ~900 ms to draw, and a
/// session of 100 is what a person reviews in one sitting.
const CARDS_PER_NOTE: usize = 100;

/// The body of the help window: the keys, what a note holds, and where the files live. A free
/// function so the QA test can draw it without a whole window.
pub(crate) fn help_body(ui: &mut egui::Ui) {
    section(ui, tl!("Getting around"));
    shortcut(ui, "Ctrl+Shift+O", tl!("Open a vault"));
    shortcut(ui, "Ctrl+Shift+F", tl!("Search the vault"));
    shortcut(ui, "Ctrl+S", tl!("Save the note"));
    shortcut(ui, "Esc", tl!("Close the panel or the window in front"));

    section(ui, tl!("Writing"));
    shortcut(ui, "Ctrl+B", tl!("Bold"));
    shortcut(ui, "Ctrl+I", tl!("Italic"));
    shortcut(ui, "Ctrl+E", tl!("Inline code"));
    shortcut(ui, "Ctrl+Shift+E", tl!("Highlight"));
    shortcut(ui, "Tab / Shift+Tab", tl!("Indent or outdent the line"));
    shortcut(ui, "Ctrl+Enter", tl!("Tick or untick a task"));
    shortcut(ui, "[[", tl!("Link to another note, with completion"));

    section(ui, tl!("What a note holds"));
    for (syntax, what) in [
        (tl!("# Title"), tl!("a heading, ## for the next level")),
        (tl!("**bold** and *italic*"), tl!("and `code`, ==highlight==, ~~struck~~")),
        (tl!("[[Note]] or [[Note#Section]]"), tl!("a link; ![[Note]] shows its text here")),
        (tl!("- [ ] task"), tl!("a task list; Ctrl+Enter ticks it")),
        (tl!("> [!warning] Title"), tl!("a callout: note, tip, warning, danger…")),
        (tl!("> [!card]- Question"), tl!("a flashcard; the answer goes in the body")),
        (tl!("> [!card]"), tl!("with ==highlights==: one cloze card per highlight")),
    ] {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(syntax).monospace().small());
            ui.label(egui::RichText::new(what).small().weak());
        });
    }

    section(ui, tl!("Where things live"));
    ui.label(egui::RichText::new(tl!("A vault is a folder of .md files. Pictures and audio go in _attachments/, your own templates in _templates/, and the review schedule in .openobsidian/srs.json - the same file the phone app reads.")).small());

    section(ui, tl!("Reviewing"));
    shortcut(ui, "Space", tl!("Show the answer"));
    shortcut(ui, "1 2 3 4", &format!("{} / {} / {} / {}", tl!("Again"), tl!("Hard"), tl!("Good"), tl!("Easy")));

    ui.separator();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("The full documentation is in the repository:")).small().weak());
        ui.hyperlink_to(egui::RichText::new(tl!("README")).small(), APP.repo_url());
    });
}

/// A section heading in the help window.
fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(egui::RichText::new(title).strong());
    ui.add_space(2.0);
}

/// One key combination and what it does.
fn shortcut(ui: &mut egui::Ui, keys: &str, what: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(keys).monospace().small());
        ui.label(egui::RichText::new(what).small());
    });
}

impl eframe::App for LocalApp {
    /// Work that is not drawing: restoring the last vault, the update check, the pickers' answers,
    /// the autosave, and the shortcuts.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Anything done here shows up on the *next* frame, and egui only redraws when asked to
        // (a static window waits for input). Opening the vault or a picked folder therefore has to
        // wake the window, or the welcome page would stay on screen until the mouse moves.
        let mut wake = false;

        if std::mem::take(&mut self.restore_pending)
            && let Some(path) = self.prefs.last_vault.clone()
        {
            self.open_vault(path);
            wake = true;
        }
        self.updater.check_at_start(ctx);
        self.updater.tick(ctx);

        for pick in self.pickers.take() {
            match pick {
                Pick::Folder(path) => self.open_vault(path),
                Pick::Document(path) => self.import_document(&path),
                Pick::Anki(path) => self.import_anki(&path),
                Pick::Save(path) => self.export_html(path),
            }
            wake = true;
        }

        if self.review.handle_keys(&mut self.vault, ctx) {
            wake = true;
        }

        // The vault folder as the OS sees it: collect the events, act when it is quiet again.
        if let Some(watcher) = &mut self.watcher
            && watcher.poll()
        {
            self.fs_change = Some(std::time::Instant::now());
            wake = true;
        }
        if crate::watcher::Watcher::settled(self.fs_change) {
            let changed = self.watcher.as_mut().map(crate::watcher::Watcher::take).unwrap_or_default();
            self.fs_change = None;
            if !changed.is_empty() {
                self.react_to_disk(&changed);
                wake = true;
            }
        }
        let save_shortcut = ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::S));
        if save_shortcut && self.vault.dirty {
            self.save();
        }
        let search_shortcut = ctx.input(|input| input.modifiers.command && input.modifiers.shift && input.key_pressed(egui::Key::F));
        if search_shortcut && self.vault.root.is_some() {
            self.toggle_search();
            wake = true;
        }
        let outline_shortcut = ctx.input(|input| input.modifiers.command && input.modifiers.shift && input.key_pressed(egui::Key::O));
        if outline_shortcut && self.vault.root.is_some() {
            self.vault.outline_open = !self.vault.outline_open;
            self.remember();
            wake = true;
        }
        let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        if escape && self.vault.search.open && !self.vault.search.query.is_empty() {
            // Escape clears the box first, closes the panel second: losing the query to a stray key
            // press is worse than one more key.
            self.vault.search.query.clear();
            self.vault.search.results.clear();
            self.vault.search.invalidate();
            wake = true;
        } else if escape && self.vault.search.open {
            self.vault.search.open = false;
            wake = true;
        }
        self.vault.refresh_search();
        if self.vault.autosave() {
            // The autosave's own write comes back through the watcher; it is not news from outside.
            if let Some(path) = self.vault.active.clone() {
                self.saved_at = Some((path, std::time::Instant::now()));
            }
            wake = true;
        }
        if wake {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        match self.frame.clone() {
            craft_window::Frame::App(layout) => self.header_bar(ui, &layout),
            // The desktop draws the frame; the menus still need a strip inside the window.
            craft_window::Frame::Desktop => {
                egui::Panel::top("menu_bar").show(ui, |ui| self.menus(ui));
            }
        }

        self.page(ui);

        if self.about_open {
            self.about(ui);
        }
        self.dialogs(ui);
        self.graph(ui);
        self.calendar(ui);
        self.help(ui);

        // The update prompt and its progress, drawn last so it sits above the window's content.
        match self.updater.show(ui.ctx()) {
            Action::None => {}
            Action::OpenUrl(url) => {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
            // The update is installed: close through the normal path, and the helper restarts us.
            Action::Close => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }
}

/// One window button of the app-drawn header bar.
fn window_button(ui: &mut egui::Ui, button: craft_window::Button) {
    let (label, tooltip) = match button {
        craft_window::Button::Minimize => ("-", tl!("Minimize")),
        craft_window::Button::Maximize => ("[]", tl!("Maximize")),
        craft_window::Button::Close => ("x", tl!("Close")),
    };
    if ui.button(label).on_hover_text(tooltip).clicked() {
        let command = match button {
            craft_window::Button::Minimize => egui::ViewportCommand::Minimized(true),
            craft_window::Button::Maximize => {
                let maximized = ui.input(|i| i.viewport().maximized).unwrap_or(false);
                egui::ViewportCommand::Maximized(!maximized)
            }
            craft_window::Button::Close => egui::ViewportCommand::Close,
        };
        ui.ctx().send_viewport_cmd(command);
    }
}
