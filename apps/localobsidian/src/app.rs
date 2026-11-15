//! The window: title bar (when the app draws it), menus, the vault page, the About window that
//! fronts the update check, and the small dialogs the vault needs (new note, rename).
//!
//! The page is the sidebar plus the note pane; the vault's own state lives in
//! [`crate::vault_state::VaultState`], and the views hand back what the user asked for.

use std::path::PathBuf;

use craft_update::updater::{Action, Status, Updater};

use crate::editor::{self, EditorAction};
use crate::i18n;
use crate::images::Images;
use crate::pickers::Pickers;
use crate::prefs::Prefs;
use crate::sidebar::{self, SidebarAction};
use crate::suite_entry::APP;
use crate::tl;
use crate::vault_state::{VaultState, ViewMode};

/// A small dialog the page opens on top of itself.
enum Dialog {
    NewNote { name: String },
    Rename { name: String },
}

pub struct LocalApp {
    updater: Updater,
    pickers: Pickers,
    /// Who draws the window frame (probed before the window was created).
    frame: craft_window::Frame,
    about_open: bool,
    /// The `language` setting: [`i18n::AUTO`] or a code from [`i18n::LANGUAGES`].
    language: String,
    prefs: Prefs,
    vault: VaultState,
    /// The pictures the reading view has decoded (`![[foto.png]]`).
    images: Images,
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

        Self {
            updater: Updater::new(&APP, env!("CARGO_PKG_VERSION")),
            pickers: Pickers::default(),
            frame,
            about_open: false,
            language: prefs.language.clone(),
            restore_pending: prefs.last_vault.is_some(),
            sidebar_open: prefs.sidebar_open,
            prefs,
            vault,
            images: Images::default(),
            dialog: None,
        }
    }

    fn remember(&self) {
        let mut prefs = self.prefs.clone();
        prefs.sort = self.vault.sort;
        prefs.view = self.vault.view;
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
                    self.dialog = Some(Dialog::NewNote { name: String::new() });
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
                if ui.add_enabled(self.vault.root.is_some(), egui::Button::new(tl!("Refresh the vault"))).clicked() {
                    self.reload();
                    ui.close();
                }
            });
            ui.menu_button(tl!("Help"), |ui| {
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
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let notes = self.vault.files.len();
                ui.label(egui::RichText::new(i18n::tr_fmt("{count} notes", &[("count", &notes.to_string())])).small().weak());
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
            Dialog::NewNote { mut name } => {
                egui::Window::new(tl!("New note"))
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
                            submitted |= ui.add_enabled(!name.trim().is_empty(), egui::Button::new(tl!("Create"))).clicked();
                            cancel |= ui.button(tl!("Cancel")).clicked();
                        });
                    });
                if submitted && !name.trim().is_empty() {
                    match self.vault.create_note(name.trim()) {
                        Ok(_) => self.remember(),
                        Err(error) => self.vault.message = Some(format!("{}: {error}", tl!("could not create the note"))),
                    }
                } else if open && !cancel {
                    self.dialog = Some(Dialog::NewNote { name });
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
            Ok(()) => self.vault.message = Some(tl!("saved").to_string()),
            Err(error) => self.vault.message = Some(format!("{}: {error}", tl!("could not save the note"))),
        }
    }

    /// Follows a `[[wikilink]]`: opens the note it names.
    fn navigate(&mut self, target: &str) {
        let parsed = localobsidian_md::parse_target(target);
        if let Some(note) = self.vault.resolve(parsed.note) {
            let path = note.path.clone();
            self.vault.open_note(&path);
            if parsed.hash.is_some() {
                // The reading view shows the whole note; scrolling to the section is a later step.
                self.vault.view = ViewMode::Read;
                self.remember();
            }
        } else {
            self.vault.message = Some(i18n::tr_fmt("no note is called {name}", &[("name", parsed.note)]));
        }
    }
}

impl eframe::App for LocalApp {
    /// Work that is not drawing: restoring the last vault, the update check, the pickers' answers,
    /// the autosave, and the shortcuts.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if std::mem::take(&mut self.restore_pending)
            && let Some(path) = self.prefs.last_vault.clone()
        {
            self.open_vault(path);
        }
        self.updater.check_at_start(ctx);
        self.updater.tick(ctx);

        for path in self.pickers.take() {
            self.open_vault(path);
        }

        let save_shortcut = ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::S));
        if save_shortcut && self.vault.dirty {
            self.save();
        }
        self.vault.autosave();
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
