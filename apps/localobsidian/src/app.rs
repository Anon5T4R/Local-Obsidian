//! The window: title bar (when the app draws it), menus, the vault page, the About window that
//! fronts the update check, and the small dialogs the vault needs (new note, rename).
//!
//! The page is the sidebar plus the note pane; the vault's own state lives in
//! [`crate::vault_state::VaultState`], and the views hand back what the user asked for.

use std::hash::{BuildHasher, Hasher};
use std::path::PathBuf;

use craft_icons::{Icon, ThemeMode};
use craft_update::updater::{Action, Status, Updater};

use crate::ai_ui::Chat;
use crate::calendar;
use crate::connections::{self, Map};
use crate::editor::{self, EditorAction};
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

mod commands;

/// A small dialog the page opens on top of itself.
enum Dialog {
    NewNote { name: String, template: String },
    Rename { name: String },
}

/// The windows the narrow-window test opens, one by one (test-only: the app never asks for them by name).
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum WindowForTest {
    NewNote,
    Rename,
    Diagnostics,
    Connections,
    Calendar,
    Help,
    About,
    Settings,
    AiConnection,
    Flashcards,
    Palette,
    Update,
}

/// The dev-only window capture (`LOCALOBSIDIAN_QA_SHOT="<png>" LOCALOBSIDIAN_QA_VAULT="<folder>"
/// LOCALOBSIDIAN_QA_NOTE="<name>" LOCALOBSIDIAN_QA_FRAMES=40`): wait for the layout to settle, write
/// one PNG of the real window and close. It is what the visual QA of a whole window looks at, and it
/// does nothing unless the variable is set.
struct QaShot {
    path: PathBuf,
    frames: u32,
    taken: bool,
}

pub struct LocalApp {
    updater: Updater,
    pickers: Pickers,
    /// Who draws the window frame (probed before the window was created).
    frame: craft_window::Frame,
    about_open: bool,
    /// The Settings window (language and theme), opened by the gear and File > Settings….
    settings_open: bool,
    /// The command palette (Ctrl+K, or the search icon of the top bar).
    palette: craft_palette::Palette,
    /// Whether the settings are written to disk (off in the headless tests: they must not touch the user's file).
    persist: bool,
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
    /// The AI flashcards action (a new note beside the one it started on).
    flashcards: crate::flashcards::Flashcards,
    /// Whether the vault diagnostics window is on screen.
    diagnostics_open: bool,
    /// Whether the connection map is on screen, and where it started.
    connections_open: bool,
    connections: Map,
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
    /// The window capture of a QA run, if one was asked for.
    qa_shot: Option<QaShot>,
    /// The chat panel and its connection.
    chat: Chat,
}

impl LocalApp {
    pub fn new(cc: &eframe::CreationContext<'_>, frame: craft_window::Frame) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        install_styles(&cc.egui_ctx);
        // Linux: the desktop's light/dark choice, which winit does not report.
        craft_desktop_theme::follow(&cc.egui_ctx);
        let prefs = Prefs::load();
        i18n::set_language(&prefs.language);
        Self::build(&cc.egui_ctx, frame, prefs)
    }

    /// The app the headless tests drive: no window, and nothing written to the user's settings file.
    #[cfg(test)]
    pub(crate) fn for_test(ctx: &egui::Context, prefs: Prefs) -> Self {
        let mut app = Self::build(ctx, craft_window::Frame::Desktop, prefs);
        app.persist = false;
        app
    }

    /// Opens a vault the way the app does at start (the QA shots need a window with notes in it).
    #[cfg(test)]
    pub(crate) fn open_vault_for_test(&mut self, path: PathBuf) {
        self.open_vault(path);
    }

    /// What the palette lists (the tests compare it with the menus).
    #[cfg(test)]
    pub(crate) fn palette_commands(&self, ctx: &egui::Context) -> Vec<craft_palette::Command> {
        self.commands(ctx)
    }

    /// What the shell tests look at.
    #[cfg(test)]
    pub(crate) fn shell_state(&self) -> (ThemeMode, bool, bool) {
        (self.prefs.theme, self.settings_open, self.palette.is_open())
    }

    /// The windows of the narrow-window test, one by one, over a vault with notes.
    #[cfg(test)]
    pub(crate) fn open_window_for_test(&mut self, window: WindowForTest) {
        match window {
            WindowForTest::NewNote => self.begin_new_note(),
            WindowForTest::Rename => self.begin_rename(),
            WindowForTest::Diagnostics => self.diagnostics_open = true,
            WindowForTest::Connections => self.set_connections(true),
            WindowForTest::Calendar => self.calendar_open = true,
            WindowForTest::Help => self.help_open = true,
            WindowForTest::About => self.about_open = true,
            WindowForTest::Settings => self.settings_open = true,
            WindowForTest::AiConnection => self.chat.settings_open = true,
            WindowForTest::Flashcards => self.start_flashcards(),
            WindowForTest::Palette => self.palette.open(),
            WindowForTest::Update => self.offer_update_for_test(),
        }
    }

    /// The update card as the UI test sees it: a release far ahead, with the launcher's version
    /// choices read from a file that does not exist (the user's own pins are not the test's).
    #[cfg(test)]
    fn offer_update_for_test(&mut self) {
        let pins = std::env::temp_dir().join(format!("localobsidian-ui-pins-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&pins);
        let mut updater = Updater::new(&APP, env!("CARGO_PKG_VERSION")).with_pins_path(pins);
        updater.offer(craft_update::Release {
            tag: "v99.0.0".to_string(),
            name: "Local Obsidian v99.0.0".to_string(),
            html_url: "https://example.invalid/local-obsidian-v99".to_string(),
            ..Default::default()
        });
        self.updater = updater;
    }

    /// The app over `prefs`, on a context whose fonts and styles are installed (the tests build it without a window).
    fn build(ctx: &egui::Context, frame: craft_window::Frame, prefs: Prefs) -> Self {
        // The SVG loader for the suite's icons (theme, settings, search).
        egui_extras::install_image_loaders(ctx);
        // The theme the settings ask for: the system's (the default) or the one the user pinned.
        apply_theme(ctx, prefs.theme);

        let mut vault = VaultState::default();
        vault.sort = prefs.sort;
        vault.view = prefs.view;
        vault.outline_open = prefs.outline_open;
        vault.tags_open = prefs.tags_open;
        vault.split = prefs.split;
        vault.zoom = prefs.zoom;

        let mut app = Self {
            updater: Updater::new(&APP, env!("CARGO_PKG_VERSION")),
            pickers: Pickers::default(),
            frame,
            about_open: false,
            settings_open: false,
            palette: craft_palette::Palette::default(),
            persist: true,
            help_open: false,
            language: prefs.language.clone(),
            restore_pending: prefs.last_vault.is_some(),
            sidebar_open: prefs.sidebar_open,
            prefs,
            vault,
            images: Images::default(),
            review: Review::default(),
            connections_open: false,
            diagnostics_open: false,
            flashcards: crate::flashcards::Flashcards::default(),
            connections: Map::default(),
            calendar_open: false,
            calendar_month: crate::calendar::today_iso(),
            watcher: None,
            fs_change: None,
            saved_at: None,
            dialog: None,
            chat: {
                let mut chat = Chat::default();
                chat.load();
                chat
            },
            qa_shot: std::env::var_os("LOCALOBSIDIAN_QA_SHOT").map(|path| QaShot {
                path: PathBuf::from(path),
                frames: std::env::var("LOCALOBSIDIAN_QA_FRAMES").ok().and_then(|frames| frames.parse().ok()).unwrap_or(40),
                taken: false,
            }),
        };

        // `LOCALOBSIDIAN_QA_VAULT` opens a vault before the first frame - the capture has no hands to
        // click with - and `LOCALOBSIDIAN_QA_NOTE` opens a note by name inside it.
        if let Ok(view) = std::env::var("LOCALOBSIDIAN_QA_VIEW") {
            app.vault.view = ViewMode::from_key(&view);
        }
        if std::env::var_os("LOCALOBSIDIAN_QA_TAGS_OPEN").is_some() {
            app.vault.tags_open = true;
        }
        // `LOCALOBSIDIAN_QA_THEME=light|dark|system` picks the theme of the capture: the machine's
        // own theme would make two runs (and two reviewers) disagree.
        if let Ok(theme) = std::env::var("LOCALOBSIDIAN_QA_THEME") {
            app.prefs.theme = ThemeMode::from_id(&theme);
            apply_theme(ctx, app.prefs.theme);
        }
        if let Some(vault) = std::env::var_os("LOCALOBSIDIAN_QA_VAULT") {
            app.restore_pending = false;
            app.open_vault(PathBuf::from(vault));
            if let Some(name) = std::env::var_os("LOCALOBSIDIAN_QA_NOTE") {
                let wanted = name.to_string_lossy().to_string();
                if let Some(note) = app.vault.files.iter().find(|note| note.name.eq_ignore_ascii_case(&wanted)) {
                    let path = note.path.clone();
                    app.vault.open_note(&path);
                }
            }
            // `LOCALOBSIDIAN_QA_CONNECTIONS=1` opens the connection map at that note: the capture
            // has no hands to reach the menu with.
            if std::env::var_os("LOCALOBSIDIAN_QA_CONNECTIONS").is_some()
                && let Some(active) = app.vault.active.clone()
            {
                app.connections_open = true;
                app.connections.start_from(&active);
            }
        }
        app
    }

    /// One picture of the real window, for the visual QA run (see [`QaShot`]).
    fn qa_capture(&mut self, ctx: &egui::Context) {
        let Some(plan) = &mut self.qa_shot else { return };
        if plan.taken {
            return;
        }
        if plan.frames > 0 {
            plan.frames -= 1;
            ctx.request_repaint();
            return;
        }
        plan.taken = true;
        let path = plan.path.clone();
        let close_ctx = ctx.clone();
        ctx.request_screenshot(move |image| {
            let [width, height] = [image.size[0] as u32, image.size[1] as u32];
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
            if let Some(folder) = path.parent() {
                let _ = std::fs::create_dir_all(folder);
            }
            match image::save_buffer(&path, &bytes, width, height, image::ExtendedColorType::Rgba8) {
                Ok(()) => log::info!("qa: wrote {} ({width}x{height})", path.display()),
                Err(error) => log::warn!("qa: cannot write {}: {error}", path.display()),
            }
            close_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        });
    }

    fn remember(&self) {
        let mut prefs = self.prefs.clone();
        prefs.sort = self.vault.sort;
        prefs.view = self.vault.view;
        prefs.outline_open = self.vault.outline_open;
        prefs.tags_open = self.vault.tags_open;
        prefs.split = self.vault.split;
        prefs.zoom = self.vault.zoom;
        prefs.sidebar_open = self.sidebar_open;
        prefs.language = self.language.clone();
        if self.persist {
            prefs.save();
        }
    }

    /// Switches to `mode`: the egui theme and the saved preference follow.
    fn set_mode(&mut self, ctx: &egui::Context, mode: ThemeMode) {
        self.prefs.theme = mode;
        apply_theme(ctx, mode);
        self.remember();
    }

    /// The suite's controls at the right of the top bar: the command palette, the theme (Automatic/Light/Dark) and the
    /// gear that opens Settings. Called inside a right-to-left layout, so the gear is drawn first and ends up rightmost.
    fn shell_buttons(&mut self, ui: &mut egui::Ui) {
        // Off the window's edge, and a finger's width between them.
        ui.add_space(8.0);
        if craft_icons::ui::button(ui, Icon::Settings, tl!("Settings")).clicked() {
            self.settings_open = true;
        }
        ui.add_space(6.0);
        let mut mode = self.prefs.theme;
        let tooltip = i18n::tr_fmt("Theme: {mode}", &[("mode", tl!(mode.label()))]);
        if craft_icons::ui::theme_button(ui, &mut mode, &tooltip) {
            self.set_mode(ui.ctx(), mode);
        }
        ui.add_space(6.0);
        let shortcut = ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::K));
        let search = i18n::tr_fmt("Search commands ({shortcut})", &[("shortcut", &shortcut)]);
        if craft_icons::ui::button(ui, Icon::Search, &search).clicked() {
            self.palette.open();
        }
    }

    /// The command palette, when open: runs what the user picked.
    fn palette(&mut self, ctx: &egui::Context) {
        if !self.palette.is_open() {
            return;
        }
        let commands = self.commands(ctx);
        let labels = craft_palette::Labels { placeholder: tl!("Search commands"), empty: tl!("No command matches.") };
        if let Some(id) = self.palette.show(ctx, &labels, &commands) {
            self.run_command(ctx, &id);
        }
    }

    /// Why a command that needs a vault cannot run right now, or `None` when it can.
    fn needs_vault(&self) -> Option<&'static str> {
        self.vault.root.is_none().then(|| tl!("Open a vault first"))
    }

    /// Why a command that needs the note in front cannot run right now.
    fn needs_note(&self) -> Option<&'static str> {
        self.vault.active.is_none().then(|| tl!("Open a note first"))
    }

    /// Why a command that opens a file dialog cannot run right now.
    fn needs_free_dialog(&self) -> Option<&'static str> {
        self.pickers.busy().then(|| tl!("Finish the open dialog first"))
    }

    /// File > New note…: the dialog that asks for the name.
    fn begin_new_note(&mut self) {
        self.dialog = Some(Dialog::NewNote { name: String::new(), template: String::new() });
    }

    /// File > Rename note…: the dialog, starting from the note's own name.
    fn begin_rename(&mut self) {
        let name = self.vault.active.as_ref().and_then(|path| path.file_stem()).map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
        self.dialog = Some(Dialog::Rename { name });
    }

    /// View > Zoom: one step of a tenth, or back to the middle.
    fn step_zoom(&mut self, step: i32) {
        self.vault.zoom = (self.vault.zoom + 0.1 * step as f32).clamp(crate::prefs::ZOOM_MIN, crate::prefs::ZOOM_MAX);
        self.remember();
    }

    /// View > Show the connections: a checkbox that opens the map at the note in front of the reader.
    fn set_connections(&mut self, open: bool) {
        self.connections_open = open;
        if open && let Some(active) = self.vault.active.clone() {
            self.connections.start_from(&active);
        }
    }

    /// AI > Create flashcards from this note (the shortcut and the palette do the same).
    fn start_flashcards(&mut self) {
        self.flashcards.start(&self.vault, None);
    }

    /// Review > the note in front, as a deck.
    fn review_note(&mut self) {
        let relative = self
            .vault
            .active
            .as_ref()
            .and_then(|path| self.vault.files.iter().find(|file| &file.path == path))
            .map(|file| file.relative_path.clone());
        if let Some(relative) = relative {
            self.review.start(&self.vault, Deck::Note(relative));
        }
    }

    /// The menu bar: the app's commands and Help.
    fn menus(&mut self, ui: &mut egui::Ui) {
        let mut quit = false;
        let ctx = ui.ctx().clone();
        let (no_vault, no_note, busy) = (self.needs_vault(), self.needs_note(), self.needs_free_dialog());
        let with_vault = no_vault.or(busy);
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tl!("File"), |ui| {
                if item(ui, tl!("Open a vault…"), "", busy).clicked() {
                    self.ask_vault(ui.ctx());
                    ui.close();
                }
                if item(ui, tl!("New note…"), "", no_vault).clicked() {
                    self.begin_new_note();
                    ui.close();
                }
                if item(ui, tl!("Import a Word or OpenDocument file…"), "", with_vault)
                    .on_hover_text(tl!("Reads a .docx or .odt into a note. Pictures are left out."))
                    .clicked()
                {
                    self.ask_document(ui.ctx());
                    ui.close();
                }
                if item(ui, tl!("Import Anki cards (text)…"), "", with_vault)
                    .on_hover_text(tl!("A text export from Anki (.txt, .tsv, .csv). A .apkg package is not read."))
                    .clicked()
                {
                    self.ask_anki(ui.ctx());
                    ui.close();
                }
                if item(ui, tl!("Rename note…"), "", no_note).clicked() {
                    self.begin_rename();
                    ui.close();
                }
                ui.separator();
                let nothing_to_save = (!self.vault.dirty).then(|| tl!("No unsaved changes"));
                if item(ui, tl!("Save"), &keys(&ctx, egui::Modifiers::COMMAND, egui::Key::S), no_note.or(nothing_to_save)).clicked() {
                    self.save();
                    ui.close();
                }
                if item(ui, tl!("Export the note as HTML…"), "", no_note.or(busy)).clicked() {
                    self.ask_export(ui.ctx());
                    ui.close();
                }
                if item(ui, tl!("Export the note as PDF…"), "", no_note.or(busy))
                    .on_hover_text(tl!("A4 pages with selectable text. Pictures and links are the next phase."))
                    .clicked()
                {
                    self.ask_export_pdf(ui.ctx());
                    ui.close();
                }
                ui.separator();
                if item(ui, tl!("Settings…"), "", None).clicked() {
                    self.settings_open = true;
                    ui.close();
                }
                ui.separator();
                if item(ui, tl!("Exit"), "", None).clicked() {
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
                ui.menu_button(tl!("Theme"), |ui| {
                    for mode in ThemeMode::EVERY {
                        if ui.radio(self.prefs.theme == mode, tl!(mode.label())).clicked() {
                            self.set_mode(ui.ctx(), mode);
                            ui.close();
                        }
                    }
                });
                ui.menu_button(tl!("Zoom"), |ui| {
                    for (label, step) in [(tl!("Zoom in"), 1), (tl!("Zoom out"), -1)] {
                        if ui.button(label).clicked() {
                            self.step_zoom(step);
                            ui.close();
                        }
                    }
                    if ui.button(tl!("Reset the zoom")).clicked() {
                        self.vault.zoom = 1.0;
                        self.remember();
                        ui.close();
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
                let search_keys = keys(&ctx, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::F);
                if item(ui, tl!("Search the vault"), &search_keys, no_vault).clicked() {
                    self.toggle_search();
                    ui.close();
                }
                if ui.checkbox(&mut self.vault.outline_open, tl!("Show the outline")).clicked() {
                    self.remember();
                    ui.close();
                }
                let mut connections_open = self.connections_open;
                if ui.checkbox(&mut connections_open, tl!("Show the connections")).clicked() {
                    self.set_connections(connections_open);
                    ui.close();
                }
                if ui.checkbox(&mut self.diagnostics_open, tl!("Vault diagnostics")).clicked() {
                    ui.close();
                }
                if item(ui, tl!("Random note"), &keys(&ctx, egui::Modifiers::COMMAND, egui::Key::R), no_vault).clicked() {
                    self.open_random_note();
                    ui.close();
                }
                if ui.checkbox(&mut self.calendar_open, tl!("Show the calendar")).clicked() {
                    ui.close();
                }
                ui.separator();
                if item(ui, tl!("Refresh the vault"), "", no_vault).clicked() {
                    self.reload();
                    ui.close();
                }
            });
            ui.menu_button(tl!("Review"), |ui| {
                if item(ui, tl!("Review the whole vault"), "", no_vault).clicked() {
                    self.review.start(&self.vault, Deck::All);
                    ui.close();
                }
                let note = self.vault.active.as_ref().and_then(|path| {
                    self.vault.files.iter().find(|file| &file.path == path).map(|file| (file.relative_path.clone(), file.name.clone()))
                });
                // The two entries are not the same thing, so they do not read the same: the note's
                // own name and how many cards it has, then the vault (which is always the bigger
                // number, and the one that used to swallow the other).
                if let Some((relative, name)) = &note {
                    let cards = self.vault.deck_cards(Some(std::slice::from_ref(relative)));
                    let label = i18n::tr_fmt("Review “{note}” ({count} cards)", &[("note", name), ("count", &cards.to_string())]);
                    if ui.button(label).on_hover_text(tl!("Only the cards of the note that is open")).clicked() {
                        self.review_note();
                        ui.close();
                    }
                } else {
                    ui.add_enabled(false, egui::Button::new(tl!("Review this note"))).on_disabled_hover_text(tl!("Open a note first"));
                }
                if let Some(tag) = self.vault.tag_filter.clone() {
                    let files: Option<Vec<String>> = self.vault.index.tags.get(&tag).map(|names| {
                        names
                            .iter()
                            .filter_map(|name| self.vault.files.iter().find(|file| file.name.eq_ignore_ascii_case(name)))
                            .map(|file| file.relative_path.clone())
                            .collect()
                    });
                    let cards = self.vault.deck_cards(files.as_deref());
                    let label = i18n::tr_fmt("Review #{tag} ({count} cards)", &[("tag", &tag), ("count", &cards.to_string())]);
                    if ui.button(label).clicked() {
                        self.review.start(&self.vault, Deck::Tag(tag));
                        ui.close();
                    }
                }
                ui.separator();
                let all = self.vault.deck_cards(None);
                let label = i18n::tr_fmt("Review the whole vault ({count} cards)", &[("count", &all.to_string())]);
                if item(ui, &label, "", no_vault).clicked() {
                    self.review.start(&self.vault, Deck::All);
                    ui.close();
                }
                if ui.button(tl!("Review statistics")).clicked() {
                    self.review.stats_open = true;
                    ui.close();
                }
                if self.review.open && ui.button(tl!("Close the review panel")).clicked() {
                    self.review.open = false;
                    ui.close();
                }
            });
            ui.menu_button(tl!("AI"), |ui| {
                if ui.checkbox(&mut self.chat.open, tl!("Chat about the note")).clicked() {
                    ui.close();
                }
                if ui.button(tl!("Connection…")).clicked() {
                    self.chat.settings_open = true;
                    ui.close();
                }
                ui.separator();
                let flashcards_keys = keys(&ctx, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::K);
                if item(ui, tl!("Create flashcards from this note"), &flashcards_keys, no_note).clicked() {
                    self.start_flashcards();
                    ui.close();
                }
            });
            ui.menu_button(tl!("Help"), |ui| {
                if ui.button(tl!("Keyboard shortcuts and help")).clicked() {
                    self.help_open = true;
                    ui.close();
                }
                ui.separator();
                if ui.button(tl!("Check for Updates…")).clicked() {
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
            // Panels push the page; they never take it. The ceiling is a share of what is left, so
            // a long note name or a wide result cannot stretch a sidebar into the window.
            let ceiling = panel_ceiling(ui.available_width(), 0.34, 380.0);
            egui::Panel::left("sidebar").resizable(true).default_size(240.0).max_size(ceiling).show(ui, |ui| {
                match sidebar::show(ui, &mut self.vault) {
                    SidebarAction::None => {}
                    SidebarAction::OpenNote(path) => self.vault.open_note(&path),
                    SidebarAction::OpenBinary(path) => self.open_with_system(&path),
                    SidebarAction::RevealFolder(path) => self.reveal_folder(&path),
                    SidebarAction::Flashcards(path) => self.flashcards.start(&self.vault, Some(&path)),
                }
            });
        }
        if self.vault.search.open {
            let ceiling = panel_ceiling(ui.available_width(), 0.5, 520.0);
            egui::Panel::right("search").resizable(true).default_size(320.0).max_size(ceiling).show(ui, |ui| {
                match panels::search(ui, &mut self.vault) {
                    PanelAction::None => {}
                    PanelAction::Open { path, line } => self.vault.show_result(&path, line),
                    PanelAction::GoToSection(section) => self.goto_section(&section),
                }
            });
        }
        if self.vault.outline_open {
            let ceiling = panel_ceiling(ui.available_width(), 0.5, 400.0);
            egui::Panel::right("outline").resizable(true).default_size(220.0).max_size(ceiling).show(ui, |ui| {
                match panels::outline(ui, &mut self.vault) {
                    PanelAction::None => {}
                    PanelAction::Open { path, line } => self.vault.show_result(&path, line),
                    PanelAction::GoToSection(section) => self.goto_section(&section),
                }
            });
        }
        if self.review.open {
            let ceiling = panel_ceiling(ui.available_width(), 0.5, 520.0);
            egui::Panel::right("review").resizable(true).default_size(320.0).max_size(ceiling).show(ui, |ui| {
                self.review.ui(ui, &mut self.vault, &mut self.images);
            });
        }
        if self.chat.open {
            let ceiling = panel_ceiling(ui.available_width(), 0.5, 560.0);
            egui::Panel::right("ai-chat").resizable(true).default_size(340.0).max_size(ceiling).show(ui, |ui| {
                self.chat.ui(ui, &mut self.vault);
            });
        }
        if self.review.stats_open {
            let ceiling = panel_ceiling(ui.available_width(), 0.5, 460.0);
            egui::Panel::right("review-stats").resizable(true).default_size(300.0).max_size(ceiling).show(ui, |ui| {
                self.review.stats_ui(ui, &mut self.vault);
            });
        }
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let notes = self.vault.files.len();
                ui.label(egui::RichText::new(i18n::trn(notes as u64, "{n} note", "{n} notes")).small().weak());
                let due = self.vault.srs_stats().due;
                if due > 0 {
                    ui.separator();
                    if ui.small_button(i18n::trn(due as u64, "{n} card due", "{n} cards due")).clicked() {
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
            EditorAction::Back => {
                self.vault.go_back();
            }
            EditorAction::Forward => {
                self.vault.go_forward();
            }
            EditorAction::Navigate(target) => self.navigate(&target),
            EditorAction::ToggleTask(index) => {
                self.vault.toggle_task(index);
            }
            EditorAction::ToggleQuery(source) => {
                if !self.vault.query_expanded.remove(&source) {
                    self.vault.query_expanded.insert(source);
                }
            }
            EditorAction::FilterTag(tag) => {
                self.vault.tag_filter = Some(tag);
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
            if ui.add_enabled(!self.pickers.busy(), egui::Button::new(tl!("Open a vault…"))).clicked() {
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

    /// Opens a note picked at random, never the one already open.
    fn open_random_note(&mut self) {
        if self.vault.files.is_empty() {
            return;
        }
        // No extra dependency for one number: the hasher's per-process seed is random enough here.
        let seed = std::collections::hash_map::RandomState::new().build_hasher().finish();
        let current = self.vault.active.as_ref().and_then(|path| self.vault.files.iter().position(|note| &note.path == path));
        let index = random_pick(self.vault.files.len(), current, seed);
        let path = self.vault.files[index].path.clone();
        self.vault.open_note(&path);
    }

    /// The vault diagnostics, in its own window.
    fn diagnostics(&mut self, ui: &mut egui::Ui) {
        if !self.diagnostics_open {
            return;
        }
        let mut open = true;
        egui::Window::new(tl!("Vault diagnostics"))
            .open(&mut open)
            .default_size([640.0, 560.0])
            .default_pos(centred(ui.ctx(), egui::vec2(640.0, 560.0)))
            .show(ui.ctx(), |ui| {
                if let Some(path) = crate::diagnostics_ui::show(ui, &self.vault) {
                    self.vault.open_note(&path);
                }
            });
        self.diagnostics_open = open;
    }

    /// The connection map of the open note, in its own window.
    fn connections(&mut self, ui: &mut egui::Ui) {
        if !self.connections_open {
            return;
        }
        let mut open = true;
        egui::Window::new(tl!("Connections"))
            .open(&mut open)
            .default_size([760.0, 600.0])
            .default_pos(centred(ui.ctx(), egui::vec2(760.0, 600.0)))
            .show(ui.ctx(), |ui| match connections::show(ui, &self.vault, &mut self.connections) {
                connections::Action::None => {}
                connections::Action::Open(path) => self.vault.open_note(&path),
                connections::Action::Start(path) => self.connections.start_from(&path),
            });
        self.connections_open = open;
    }

    /// The calendar window: a day with a note opens it, a day without creates it.
    fn calendar(&mut self, ui: &mut egui::Ui) {
        if !self.calendar_open {
            return;
        }
        let mut open = true;
        egui::Window::new(tl!("Calendar"))
            .open(&mut open)
            .default_size([330.0, 340.0])
            .default_pos(centred(ui.ctx(), egui::vec2(330.0, 340.0)))
            .resizable(false)
            .show(ui.ctx(), |ui| {
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

    /// Asks where to write the open note as a PDF.
    fn ask_export_pdf(&mut self, ctx: &egui::Context) {
        let Some(active) = self.vault.active.clone() else { return };
        let name = active.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
        let name = if name.is_empty() { tl!("note").to_string() } else { name };
        if !self.pickers.ask_pdf(ctx, tl!("Export as PDF"), &format!("{name}.pdf")) {
            log::debug!("a picker is already open");
        }
    }

    /// Writes the open note as a PDF and says how many pages it made.
    fn export_pdf(&mut self, path: std::path::PathBuf) {
        let Some(active) = self.vault.active.clone() else {
            self.vault.message = Some(tl!("open a note first").to_string());
            return;
        };
        let title = active.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
        self.vault.refresh_preview();
        let blocks = self.vault.preview().to_vec();
        match export_note_pdf(&blocks, &title, &self.vault, &path) {
            Ok(report) => {
                let pages = report.pages.to_string();
                self.vault.message =
                    Some(i18n::tr_fmt("{pages} pages written to {path}", &[("pages", &pages), ("path", &path.display().to_string())]));
            }
            Err(error) => self.vault.message = Some(format!("{}: {error}", tl!("could not write the PDF"))),
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
            message.push_str(&i18n::trn(copied as u64, "{n} image copied", "{n} images copied"));
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
        egui::Window::new(tl!("Help")).open(&mut open).default_size([560.0, 540.0]).default_pos(centred(ui.ctx(), egui::vec2(560.0, 540.0))).show(
            ui.ctx(),
            |ui| {
                egui::ScrollArea::vertical().id_salt("help-body").auto_shrink([false, false]).show(ui, help_body);
            },
        );
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
                    if ui.add_enabled(!checking, egui::Button::new(tl!("Check for Updates…"))).clicked() {
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
            });
        self.about_open = open;
    }

    /// Settings: the interface language and the theme (the suite keeps both here; the gear and File > Settings…
    /// open it).
    fn settings(&mut self, ui: &mut egui::Ui) {
        let mut open = self.settings_open;
        let mut mode = self.prefs.theme;
        egui::Window::new(tl!("Settings"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                egui::Grid::new("settings").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    self.language_picker(ui);
                    ui.end_row();
                    ui.label(tl!("Theme"));
                    ui.horizontal(|ui| {
                        for choice in ThemeMode::EVERY {
                            ui.selectable_value(&mut mode, choice, tl!(choice.label()));
                        }
                    });
                    ui.end_row();
                });
            });
        if mode != self.prefs.theme {
            self.set_mode(ui.ctx(), mode);
        }
        self.settings_open = open;
    }

    /// The language row of Settings: the label, then the selector (two grid cells), straight from
    /// [`i18n::LANGUAGES`].
    fn language_picker(&mut self, ui: &mut egui::Ui) {
        ui.label(tl!("Language"));
        ui.horizontal(|ui| {
            let current = i18n::code();
            let combo = egui::ComboBox::from_id_salt("language").selected_text(i18n::name_of(current)).show_ui(ui, |ui| {
                for (code, name) in i18n::languages() {
                    if ui.selectable_label(code == current, name).clicked() && code != current {
                        self.language = code.to_string();
                        i18n::set_lang(i18n::resolve(&self.language));
                        self.remember();
                    }
                }
            });
            // A combo box with no label is a widget a screen reader cannot announce.
            combo.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, tl!("Language")));
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
                    self.shell_buttons(ui);
                });
            });
        });
    }

    fn ask_vault(&mut self, ctx: &egui::Context) {
        let start = self.prefs.last_vault.as_ref().map(|path| path.display().to_string());
        if !self.pickers.ask_folder(ctx, tl!("Open a vault…"), start.as_deref()) {
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

    /// Opens a document with the desktop's own handler (a PDF is not a note).
    fn open_with_system(&mut self, path: &std::path::Path) {
        if let Err(error) = crate::opener::open(path) {
            self.vault.message = Some(format!("{}: {error}", tl!("could not open the file")));
        }
    }

    /// Shows a folder in the file manager.
    fn reveal_folder(&mut self, path: &std::path::Path) {
        if let Err(error) = crate::opener::reveal(path) {
            self.vault.message = Some(format!("{}: {error}", tl!("could not open the folder")));
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
    shortcut(ui, "Ctrl+K", tl!("Search commands"));
    shortcut(ui, "Ctrl+Shift+O", tl!("Show the outline"));
    shortcut(ui, "Ctrl+Shift+F", tl!("Search the vault"));
    shortcut(ui, "Ctrl+S", tl!("Save the note"));
    shortcut(ui, "Ctrl+F", tl!("Find in the note being read"));
    shortcut(ui, "Ctrl+R", tl!("Open a note at random"));
    shortcut(ui, "Ctrl+Shift+K", tl!("Create flashcards from the note being read"));
    shortcut(ui, "Alt+Left / Alt+Right", tl!("Go back or forward between the notes you visited"));
    shortcut(ui, "Esc", tl!("Close the panel or the window in front"));

    section(ui, tl!("Writing"));
    shortcut(ui, "Ctrl+B", tl!("Bold"));
    shortcut(ui, "Ctrl+I", tl!("Italic"));
    shortcut(ui, "Ctrl+E", tl!("Inline code"));
    shortcut(ui, "Ctrl+Shift+E", tl!("Highlight"));
    shortcut(ui, "Tab / Shift+Tab", tl!("Indent or outdent the line"));
    shortcut(ui, "Ctrl+Enter", tl!("Tick or untick a task"));
    shortcut(ui, "[[", tl!("Link to another note, with completion"));
    shortcut(ui, "/", tl!("A block at the start of a line: table, callout, code, task, contents, date"));

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

/// The one colour the note itself carries (links, tags, quotes): the icon's violet, darkened for a
/// light background and lightened for a dark one so it stays readable either way.
///
/// Installed on **both** styles, so switching the theme never loses the accent.
pub(crate) fn install_styles(ctx: &egui::Context) {
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.visuals.hyperlink_color = crate::theme::ACCENT_DARK;
    });
    ctx.style_mut_of(egui::Theme::Light, |style| {
        style.visuals.hyperlink_color = crate::theme::ACCENT_LIGHT;
    });
}

/// Applies the theme the settings ask for. `System` follows the desktop's own light/dark choice; the
/// per-theme accent (links, tags, quotes) is already installed on both styles in [`LocalApp::new`].
pub(crate) fn apply_theme(ctx: &egui::Context, mode: ThemeMode) {
    ctx.set_theme(match mode {
        ThemeMode::System => egui::ThemePreference::System,
        ThemeMode::Light => egui::ThemePreference::Light,
        ThemeMode::Dark => egui::ThemePreference::Dark,
    });
}

/// The faces a PDF is written with: egui's bundled Latin pair, plus the craft-fonts faces the app
/// already embeds (CJK, Arabic, Hebrew, Devanagari) as fallbacks.
fn pdf_faces() -> localobsidian_pdf::Faces {
    use localobsidian_pdf::Face;
    let fallbacks = crate::fonts::CRAFT_FONTS.iter().map(|font| Face::new(font.bytes)).collect();
    localobsidian_pdf::Faces {
        regular: Face::new(epaint_default_fonts::UBUNTU_LIGHT),
        mono: Face::new(epaint_default_fonts::HACK_REGULAR),
        fallbacks,
    }
}

/// Writes the reading view's blocks to `path` as a PDF: what the menu does, without the menu (the
/// tests and the QA use it). The pictures the note embeds are read from the vault, at the paths
/// the preview resolves them by.
pub(crate) fn export_note_pdf(
    blocks: &[localobsidian_md::preview::Block],
    title: &str,
    vault: &VaultState,
    path: &std::path::Path,
) -> Result<localobsidian_pdf::Report, localobsidian_pdf::Error> {
    let images = |destination: &str| vault.attachment_bytes(destination);
    localobsidian_pdf::export(blocks, title, &pdf_faces(), &images, path)
}

/// Which note a random open picks: any of them, never the one already open.
///
/// `seed` is only a number the caller brings (the clock or a hasher); the rule that the current note
/// is skipped lives here, where it can be tested.
fn random_pick(len: usize, current: Option<usize>, seed: u64) -> usize {
    if len == 0 {
        return 0;
    }
    let index = (seed as usize) % len;
    match current {
        Some(current) if len > 1 && index == current => (index + 1) % len,
        _ => index,
    }
}

/// The widest a side panel may get: a share of the room left, never more than `absolute`.
///
/// A resizable panel grows to fit its content, and content that asks for `f32::INFINITY` (a text
/// field that wants the row, a row of results) made the panel the page: the note disappeared under
/// it and the input stretched to the edge. The ceiling is what keeps a sidebar a sidebar.
fn panel_ceiling(available: f32, share: f32, absolute: f32) -> f32 {
    (available * share).clamp(160.0, absolute)
}

/// Where a window of `size` starts so that it sits in the middle of the app.
///
/// The tool windows (graph, calendar, help) are movable, so this is only the first position: it
/// keeps them from opening over the corner the user is reading.
fn centred(ctx: &egui::Context, size: egui::Vec2) -> egui::Pos2 {
    let middle = ctx.content_rect().center();
    egui::pos2(middle.x - size.x / 2.0, middle.y - size.y / 2.0)
}

/// A shortcut as the menus and the palette show it (`Ctrl+S`, `⌘S`).
fn keys(ctx: &egui::Context, modifiers: egui::Modifiers, key: egui::Key) -> String {
    ctx.format_shortcut(&egui::KeyboardShortcut::new(modifiers, key))
}

/// A menu item: its name, the shortcut beside it, and - when it cannot run - why, as the hover of the greyed item.
fn item(ui: &mut egui::Ui, label: &str, shortcut: &str, off: Option<&str>) -> egui::Response {
    let mut button = egui::Button::new(label);
    if !shortcut.is_empty() {
        button = button.shortcut_text(shortcut);
    }
    let response = ui.add_enabled(off.is_none(), button);
    match off {
        Some(reason) => response.on_disabled_hover_text(reason),
        None => response,
    }
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
        self.run_logic(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

impl LocalApp {
    /// What `logic` does (a method of its own so the tests can run it without an `eframe::Frame`).
    pub(crate) fn run_logic(&mut self, ctx: &egui::Context) {
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
                Pick::SavePdf(path) => self.export_pdf(path),
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
        // Back and forward: the mouse's side buttons and `Alt+←`/`Alt+→`, the same pair every
        // browser has. Only when a note is open.
        if self.vault.active.is_some() {
            match crate::history::requested(ctx) {
                Some(crate::history::Direction::Back) => {
                    self.vault.go_back();
                    // The editor must not also move the caret with the same key press.
                    ctx.input_mut(|input| input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft));
                    wake = true;
                }
                Some(crate::history::Direction::Forward) => {
                    self.vault.go_forward();
                    ctx.input_mut(|input| input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowRight));
                    wake = true;
                }
                None => {}
            }
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
        // Zoom of the reading view: `Ctrl+=`, `Ctrl+-` and `Ctrl+0` for the middle. Consumed before
        // the note sees the key, or the editor would insert the character.
        let zoom_step = ctx.input(|input| {
            if !input.modifiers.command {
                return 0;
            }
            if input.key_pressed(egui::Key::Equals) || input.key_pressed(egui::Key::Plus) {
                1
            } else if input.key_pressed(egui::Key::Minus) {
                -1
            } else {
                0
            }
        });
        if zoom_step != 0 {
            self.vault.zoom = (self.vault.zoom + 0.1 * zoom_step as f32).clamp(crate::prefs::ZOOM_MIN, crate::prefs::ZOOM_MAX);
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Equals));
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Minus));
            wake = true;
        }
        if ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::Num0)) {
            self.vault.zoom = 1.0;
            wake = true;
        }
        // Find in the note being read: `Ctrl+F` opens the bar over the reading view (`Ctrl+Shift+F`
        // is the vault search). Nothing to scroll in the editor, so the key is ignored there.
        if ctx.input(|input| input.modifiers.command && !input.modifiers.shift && input.key_pressed(egui::Key::F))
            && self.vault.view.shows_reading()
            && self.vault.active.is_some()
        {
            self.vault.find.open();
            wake = true;
        }
        // A note at random: the reader's "get me somewhere I have not been today".
        if ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::R)) && self.vault.root.is_some() {
            self.open_random_note();
            wake = true;
        }
        // Flashcards from the note in front of the reader: the Ação-Artefato entry, same as the
        // menu and the tree's context menu.
        // Consumed before Ctrl+K below: egui matches Ctrl+K with Shift held too, and that is the palette.
        let flashcards_shortcut = ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::K));
        if flashcards_shortcut && self.vault.active.is_some() {
            self.start_flashcards();
            wake = true;
        }
        // The command palette: Ctrl+K opens it, and the same key closes it.
        if craft_palette::Palette::shortcut_pressed(ctx) {
            self.palette.toggle();
            wake = true;
        }
        let mut escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        if escape && self.vault.find.open {
            // The find bar closes before anything else looks at Escape.
            self.vault.find.close();
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            escape = false;
            wake = true;
        }
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
        self.chat.poll();
        // The divider and the panes are the user's; remember them once the hand lets go, not on
        // every frame of a drag (the settings file would be rewritten sixty times a second).
        let dragging = ctx.input(|input| input.pointer.primary_down());
        if !dragging && (self.vault.split != self.prefs.split || self.vault.view != self.prefs.view || self.vault.tags_open != self.prefs.tags_open) {
            self.remember();
        }
        self.qa_capture(ctx);
        if wake {
            ctx.request_repaint();
        }
    }

    /// What `ui` draws: the top bar, the page and the windows over it.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        match self.frame.clone() {
            craft_window::Frame::App(layout) => self.header_bar(ui, &layout),
            // The desktop draws the frame; the menus still need a strip inside the window.
            craft_window::Frame::Desktop => {
                egui::Panel::top("menu_bar").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        self.menus(ui);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.shell_buttons(ui));
                    });
                });
            }
        }

        self.page(ui);

        if self.about_open {
            self.about(ui);
        }
        if self.settings_open {
            self.settings(ui);
        }
        self.dialogs(ui);
        let _ = self.chat.settings(ui);
        self.flashcards.dialog(ui, &mut self.vault, &self.chat);
        self.connections(ui);
        self.diagnostics(ui);
        self.calendar(ui);
        self.help(ui);
        self.palette(ui.ctx());

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_random_note_is_never_the_one_already_open() {
        // Every seed, on a vault of three: the note in front is skipped, and the pick stays in range.
        for seed in 0..300u64 {
            let pick = random_pick(3, Some(1), seed);
            assert!(pick < 3, "seed {seed} left the vault");
            assert_ne!(pick, 1, "seed {seed} picked the note already open");
        }
        assert_eq!(random_pick(1, Some(0), 7), 0, "one note: it is the only answer");
        assert_eq!(random_pick(0, None, 7), 0, "no notes: nothing to break");
        assert_eq!(random_pick(3, None, 7), 1, "no note open: any of them by the seed");
    }
}
