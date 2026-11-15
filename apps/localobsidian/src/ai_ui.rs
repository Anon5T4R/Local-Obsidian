//! The AI panel: one chat beside the note, over the suite's BYOK client.
//!
//! What the panel does is deliberately small: it sends the note (or nothing, if none is open) as
//! context, streams the answer into view, and writes the answer into the note as text when asked.
//! The provider is the user's: a base URL (Ollama and `llama-server` are a base URL), a model from
//! the provider's own list, and a key in the OS keychain - there is no server of ours in the path.
//!
//! The answer arrives on a worker thread ([`localobsidian_ai::Stream`]); this module only polls.

use std::sync::mpsc::TryRecvError;

use localobsidian_ai as ai;

use crate::i18n;
use crate::tl;
use crate::vault_state::VaultState;

/// How much of the note goes as context. A book-length note would be refused by most providers, and
/// the tail is what the user is usually looking at.
const CONTEXT_LIMIT: usize = 12_000;

/// The width of the connection dialog: one form, one measure, centred.
pub const DIALOG_WIDTH: f32 = 520.0;

/// Who said it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    User,
    Assistant,
}

/// The chat beside the note.
pub struct Chat {
    pub open: bool,
    pub settings_open: bool,
    /// The connection in use, loaded from the suite's store.
    connection: Option<ai::Connection>,
    /// What the settings window is editing (base URL, model, key).
    pub draft_base: String,
    pub draft_model: String,
    pub draft_key: String,
    pub draft_provider: String,
    /// A model list fetched from the provider, waiting for the settings window.
    models_job: Option<std::sync::mpsc::Receiver<Result<Vec<String>, String>>>,
    pub models: Vec<String>,
    messages: Vec<(Who, String)>,
    input: String,
    /// The answer being streamed, piece by piece.
    answer: String,
    stream: Option<ai::Stream>,
    pub error: Option<String>,
    /// The note text last sent as context, kept for the header's count.
    context_chars: usize,
}

impl Default for Chat {
    fn default() -> Self {
        Self {
            open: false,
            settings_open: false,
            connection: None,
            draft_base: String::new(),
            draft_model: String::new(),
            draft_key: String::new(),
            draft_provider: "custom".to_string(),
            models_job: None,
            models: Vec::new(),
            messages: Vec::new(),
            input: String::new(),
            answer: String::new(),
            stream: None,
            error: None,
            context_chars: 0,
        }
    }
}

impl Chat {
    /// Loads the saved connection, if there is one.
    pub fn load(&mut self) {
        match ai::connections() {
            Ok(all) => {
                self.connection = all.into_iter().next();
                self.fill_draft();
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Whether the panel has somewhere to send a question.
    pub fn ready(&self) -> bool {
        self.connection.as_ref().is_some_and(|conn| !conn.model.trim().is_empty())
    }

    pub fn model(&self) -> &str {
        self.connection.as_ref().map(|conn| conn.model.as_str()).unwrap_or_default()
    }

    pub fn busy(&self) -> bool {
        self.stream.is_some()
    }

    pub fn context_chars(&self) -> usize {
        self.context_chars
    }

    /// Copies the saved connection into the settings fields.
    fn fill_draft(&mut self) {
        let Some(conn) = &self.connection else {
            self.draft_provider = "custom".to_string();
            self.draft_base = String::new();
            self.draft_model = String::new();
            return;
        };
        self.draft_provider = conn.provider.clone();
        self.draft_base = conn.base_url.clone();
        self.draft_model = conn.model.clone();
        self.draft_key = conn.key(&ai::APP).unwrap_or_default();
    }

    /// Saves what the settings window holds as the one connection this app uses.
    pub fn save_settings(&mut self) {
        let base = self.draft_base.trim().to_string();
        let model = self.draft_model.trim().to_string();
        if model.is_empty() {
            self.error = Some(tl!("choose a model first").to_string());
            return;
        }
        let provider = self.draft_provider.trim().to_string();
        let conn = ai::Connection { id: "default".to_string(), provider, label: "Local Obsidian".to_string(), base_url: base, model };
        if let Err(error) = conn.set_key(&ai::APP, self.draft_key.trim()) {
            log::warn!("ai: cannot store the key: {error}");
            self.error = Some(error);
            return;
        }
        match ai::save(std::slice::from_ref(&conn)) {
            Ok(()) => {
                self.messages.clear();
                self.answer.clear();
                self.error = None;
                self.connection = Some(conn);
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Asks the provider for its model list, on a worker thread.
    pub fn fetch_models(&mut self) {
        let conn = ai::Connection {
            id: "draft".to_string(),
            provider: self.draft_provider.clone(),
            label: String::new(),
            base_url: self.draft_base.trim().to_string(),
            model: self.draft_model.trim().to_string(),
        };
        // The key is not saved yet: the shared client reads it from the keychain, so trial it with
        // the value in the field by storing it first.
        if let Err(error) = conn.set_key(&ai::APP, self.draft_key.trim()) {
            self.error = Some(error);
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("ai-models".into())
            .spawn(move || {
                let _ = sender.send(ai::models(&conn));
            })
            .map_err(|error| log::warn!("ai: cannot start the models thread: {error}"))
            .ok();
        self.models_job = Some(receiver);
    }

    /// Sends what is in the box, with the open note as context.
    pub fn send(&mut self, state: &VaultState) {
        let Some(conn) = self.connection.clone() else {
            self.error = Some(tl!("configure the connection first").to_string());
            return;
        };
        let question = self.input.trim().to_string();
        if question.is_empty() || self.busy() {
            return;
        }
        let context = context_of(state);
        self.context_chars = context.chars().count();
        let mut history = vec![ai::Msg::System(format!(
            "You are a writing assistant inside a Markdown notes app. Answer in the language of the \
             question, in plain Markdown, without preamble.{}",
            if context.is_empty() { String::new() } else { format!("\n\nThe open note follows.\n\n{context}") }
        ))];
        for (who, text) in &self.messages {
            history.push(match who {
                Who::User => ai::Msg::User(text.clone()),
                Who::Assistant => ai::Msg::Assistant(text.clone()),
            });
        }
        history.push(ai::Msg::User(question.clone()));
        self.messages.push((Who::User, question));
        self.input.clear();
        self.answer.clear();
        self.error = None;
        self.stream = Some(ai::Stream::start(conn, history));
    }

    /// Stops a streaming answer; the panel keeps what arrived.
    pub fn stop(&mut self) {
        if let Some(stream) = &self.stream {
            stream.cancel();
        }
        if !self.answer.is_empty() {
            let answer = std::mem::take(&mut self.answer);
            self.messages.push((Who::Assistant, answer));
        }
        self.stream = None;
    }

    /// What arrived since the last frame. Called once per frame from the app's own loop.
    pub fn poll(&mut self) {
        if let Some(job) = &self.models_job {
            match job.try_recv() {
                Ok(Ok(models)) => {
                    self.models = models;
                    self.models_job = None;
                }
                Ok(Err(error)) => {
                    self.error = Some(error);
                    self.models_job = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.models_job = None,
            }
        }
        let Some(stream) = &self.stream else { return };
        let (pieces, done) = stream.poll();
        for piece in pieces {
            self.answer.push_str(&piece);
        }
        if let Some(result) = done {
            match result {
                Ok(answer) => {
                    let text = std::mem::take(&mut self.answer);
                    // A provider may send nothing in `delta` and the whole text at the end; the
                    // joined answer is what gets kept either way.
                    let text = if text.is_empty() { answer } else { text };
                    self.messages.push((Who::Assistant, text));
                }
                Err(error) => {
                    self.error = Some(error);
                    if !self.answer.is_empty() {
                        let answer = std::mem::take(&mut self.answer);
                        self.messages.push((Who::Assistant, answer));
                    }
                }
            }
            self.stream = None;
        }
    }

    /// Closes the panel, stopping whatever was running.
    pub fn close(&mut self) {
        self.stop();
        self.open = false;
    }

    /// Writes the last answer into the open note, at its end, and leaves the note in the editor.
    pub fn insert_answer(&mut self, state: &mut VaultState) -> bool {
        let Some((Who::Assistant, answer)) = self.messages.last() else { return false };
        let answer = answer.clone();
        if state.active.is_none() || answer.trim().is_empty() {
            return false;
        }
        let mut text = state.buffer.clone();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push('\n');
        text.push_str(answer.trim());
        text.push('\n');
        state.replace_buffer(text);
        state.view = crate::vault_state::ViewMode::Split;
        true
    }
}

/// The note's text, cut to a size a provider will accept.
fn context_of(state: &VaultState) -> String {
    if state.active.is_none() {
        return String::new();
    }
    let text = state.buffer.clone();
    if text.chars().count() <= CONTEXT_LIMIT {
        return text;
    }
    let start = text.char_indices().nth(text.chars().count() - CONTEXT_LIMIT).map(|(index, _)| index).unwrap_or(0);
    format!("[…]\n{}", &text[start..])
}

impl Chat {
    /// Draws the panel beside the note.
    pub fn ui(&mut self, ui: &mut egui::Ui, state: &mut VaultState) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("AI")).strong());
            let model = self.model().to_string();
            if model.is_empty() {
                ui.label(egui::RichText::new(tl!("not configured")).small().weak());
            } else {
                ui.label(egui::RichText::new(model).small().weak());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("x").on_hover_text(tl!("Close")).clicked() {
                    self.close();
                }
                if ui.small_button(tl!("Settings")).clicked() {
                    self.settings_open = true;
                }
                if !self.messages.is_empty() && ui.small_button(tl!("Clear")).clicked() {
                    self.messages.clear();
                    self.answer.clear();
                }
            });
        });
        ui.separator();

        let height = (ui.available_height() - 92.0).max(120.0);
        egui::ScrollArea::vertical().id_salt("ai-chat").max_height(height).auto_shrink([false, false]).show(ui, |ui| {
            if self.messages.is_empty() && self.answer.is_empty() {
                ui.label(egui::RichText::new(tl!("Ask about this note, or about anything.")).weak());
            }
            for (who, text) in &self.messages {
                message(ui, *who, text);
            }
            if !self.answer.is_empty() {
                message(ui, Who::Assistant, &self.answer);
            }
        });

        if let Some(error) = &self.error {
            ui.label(egui::RichText::new(error).small().color(ui.visuals().warn_fg_color));
        }
        if self.context_chars() > 0 && self.ready() {
            ui.label(
                egui::RichText::new(i18n::tr_fmt("the note goes as context: {count} characters", &[("count", &self.context_chars().to_string())]))
                    .small()
                    .weak(),
            );
        }

        let send = {
            let hint = if self.ready() { tl!("Ask about this note...") } else { tl!("configure the connection first") };
            let box_ui = ui.add(egui::TextEdit::multiline(&mut self.input).hint_text(hint).desired_rows(3).desired_width(ui.available_width()));
            box_ui.has_focus() && ui.input(|input| input.modifiers.command && input.key_pressed(egui::Key::Enter))
        };
        ui.horizontal(|ui| {
            let can_send = self.ready() && !self.input.trim().is_empty() && !self.busy();
            if ui.add_enabled(can_send, egui::Button::new(tl!("Send"))).clicked() || (send && can_send) {
                self.send(state);
            }
            if self.busy() && ui.button(tl!("Stop")).clicked() {
                self.stop();
            }
            let has_answer = matches!(self.messages.last(), Some((Who::Assistant, _)));
            if ui.add_enabled(has_answer && !self.busy(), egui::Button::new(tl!("Insert into the note"))).clicked() {
                self.insert_answer(state);
            }
        });
    }

    /// The connection window: base URL, model and key. One connection, the one this app uses.
    ///
    /// A dialog of one measured width, centred: a settings form that spans the screen is both
    /// unreadable (fields 1900 px long) and a lie about what it belongs to. Returns the window's
    /// rect, which is what the sizing test asserts on.
    pub fn settings(&mut self, ui: &mut egui::Ui) -> Option<egui::Rect> {
        if !self.settings_open {
            return None;
        }
        let mut open = true;
        let mut save = false;
        let window = egui::Window::new(tl!("AI connection"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .min_width(DIALOG_WIDTH)
            .max_width(DIALOG_WIDTH)
            .show(ui.ctx(), |ui| {
                // One width for the whole form: the field asks for the dialog, not for the screen.
                let field = ui.available_width();
                ui.label(egui::RichText::new(tl!("Provider")).small().weak());
                let current = ai::provider(&self.draft_provider).map(|preset| preset.label).unwrap_or("Custom (OpenAI-compatible)");
                egui::ComboBox::from_id_salt("ai-provider").selected_text(current).width(field).show_ui(ui, |ui| {
                    for preset in ai::PROVIDERS {
                        if ui.selectable_label(preset.id == self.draft_provider, preset.label).clicked() {
                            self.draft_provider = preset.id.to_string();
                            // A preset is a starting point: the URL stays editable after choosing one.
                            if !preset.base_url.is_empty() {
                                self.draft_base = preset.base_url.to_string();
                            }
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label(egui::RichText::new(tl!("Base URL")).small().weak());
                ui.add(egui::TextEdit::singleline(&mut self.draft_base).hint_text("http://localhost:11434/v1").desired_width(field));
                ui.label(egui::RichText::new(tl!("Ollama and llama-server are a base URL and a model.")).small().weak());
                ui.add_space(4.0);
                ui.label(egui::RichText::new(tl!("Model")).small().weak());
                ui.horizontal(|ui| {
                    // The row minus the button that sits beside it.
                    let list = 96.0;
                    ui.add(egui::TextEdit::singleline(&mut self.draft_model).desired_width((field - list).max(120.0)));
                    if ui.button(tl!("List models")).clicked() {
                        self.fetch_models();
                    }
                });
                if !self.models.is_empty() {
                    egui::ScrollArea::vertical().max_height(120.0).id_salt("ai-models").show(ui, |ui| {
                        for model in self.models.clone() {
                            if ui.selectable_label(self.draft_model == model, &model).clicked() {
                                self.draft_model = model.clone();
                            }
                        }
                    });
                }
                ui.add_space(4.0);
                ui.label(egui::RichText::new(tl!("API key")).small().weak());
                ui.add(egui::TextEdit::singleline(&mut self.draft_key).password(true).desired_width(field));
                ui.label(egui::RichText::new(tl!("Stored in the system keychain, never in a file.")).small().weak());
                if let Some(error) = &self.error {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(error).small().color(ui.visuals().warn_fg_color));
                }
                ui.add_space(10.0);
                // Align::Min, not Center: a row centred in the space that is left grows the dialog
                // to the height of the screen and floats the buttons in the middle of nothing.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    save = ui.button(tl!("Save")).clicked();
                    if ui.button(tl!("Cancel")).clicked() {
                        self.settings_open = false;
                    }
                });
            });
        if save {
            self.save_settings();
            if self.error.is_none() {
                self.settings_open = false;
            }
        } else if !open {
            self.settings_open = false;
        }
        window.map(|window| window.response.rect)
    }
}

/// One message in the transcript: who said it, and its text.
fn message(ui: &mut egui::Ui, who: Who, text: &str) {
    let (label, colour) = match who {
        Who::User => (tl!("You"), ui.visuals().text_color()),
        Who::Assistant => (tl!("AI"), crate::theme::accent(ui.visuals().dark_mode)),
    };
    ui.add_space(4.0);
    ui.label(egui::RichText::new(label).small().strong().color(colour));
    ui.label(egui::RichText::new(text).size(14.0));
}
#[cfg(test)]
mod tests {
    use super::*;

    /// A vault of one note on disk, removed when the test ends.
    struct Scratch(std::path::PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn demo(tag: &str) -> (Scratch, VaultState) {
        let root = std::env::temp_dir().join(format!("localobsidian-ai-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch dir");
        std::fs::write(root.join("Sepse.md"), "# Sepse\n\nLactato na admissão.\n").expect("the note");
        let mut state = VaultState::default();
        state.open(root.clone()).expect("open the vault");
        (Scratch(root), state)
    }

    fn chat_with_answer(answer: &str) -> Chat {
        let mut chat = Chat::default();
        chat.messages.push((Who::User, "pergunta".to_string()));
        chat.messages.push((Who::Assistant, answer.to_string()));
        chat
    }

    #[test]
    fn the_answer_is_written_at_the_end_of_the_note() {
        let (scratch, mut state) = demo("insert");
        state.open_note(&scratch.0.join("Sepse.md"));
        let mut chat = chat_with_answer("Use lactato.");
        assert!(chat.insert_answer(&mut state));
        assert!(state.buffer.ends_with("Use lactato.\n"), "{}", state.buffer);
        assert!(state.dirty, "inserting is an edit");
    }

    #[test]
    fn an_answer_without_an_open_note_is_not_written() {
        let (scratch, mut state) = demo("none");
        let _ = &scratch;
        assert!(state.active.is_none(), "nothing is open until a note is chosen");
        let mut chat = chat_with_answer("texto");
        assert!(!chat.insert_answer(&mut state));
    }

    #[test]
    fn the_context_is_the_open_note_and_nothing_else() {
        let (scratch, mut state) = demo("context");
        assert!(context_of(&state).is_empty(), "no note, no context");
        state.open_note(&scratch.0.join("Sepse.md"));
        assert!(context_of(&state).contains("Lactato"));
    }

    #[test]
    fn a_long_note_is_cut_from_the_front() {
        let (scratch, mut state) = demo("long");
        state.open_note(&scratch.0.join("Sepse.md"));
        state.buffer = format!("INÍCIO\n{}\nFIM", "x".repeat(CONTEXT_LIMIT + 500));
        let context = context_of(&state);
        assert!(context.starts_with("[…]"), "the cut is marked");
        assert!(context.ends_with("FIM"), "the tail is what stays");
        assert!(context.chars().count() <= CONTEXT_LIMIT + 8);
    }

    #[test]
    fn an_unconfigured_panel_says_so_instead_of_sending() {
        let (_scratch, state) = demo("unconfigured");
        let mut chat = Chat::default();
        assert!(!chat.ready());
        chat.input.push_str("pergunta");
        chat.send(&state);
        assert!(chat.error.is_some(), "no connection, an error the user can read");
        assert!(chat.messages.is_empty(), "nothing was sent, nothing was kept");
    }

    #[test]
    fn the_settings_need_a_model() {
        let mut chat = Chat { draft_base: "http://localhost:11434/v1".to_string(), ..Chat::default() };
        chat.save_settings();
        assert!(chat.error.is_some(), "a connection without a model cannot answer");
        assert!(!chat.ready());
    }
}
