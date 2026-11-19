//! The AI panel: the suite's chat panel and connections form, over this app's BYOK client.
//!
//! The experience - the roll, the composer, the strip, the connections form and what actually goes
//! to the model - is `craft-ai-ui`, shared with the other apps of the suite. What lives here is the
//! app's side of the contract: the system prompt, the note as context, writing an answer into the
//! note, and driving the shared `local-ai` client with what the panel hands over.

use craft_ai_ui as ui;
use localobsidian_ai as ai;

use crate::i18n;
use crate::tl;
use crate::vault_state::VaultState;

/// How much of the note goes as context. A book-length note would be refused by most providers, and
/// the tail is what the user is usually looking at.
const CONTEXT_LIMIT: usize = 12_000;

/// The whole request budget: the turns plus the note.
const BUDGET_CHARS: usize = 24_000;

/// The width of the connection dialog, from the shared component (the flashcards dialog uses it too).
pub const DIALOG_WIDTH: f32 = ui::ConnectionsPanel::WIDTH;

/// The chat beside the note.
pub struct Chat {
    pub open: bool,
    pub settings_open: bool,
    panel: ui::ChatPanel,
    strip: ui::StatusStrip,
    connections: ui::ConnectionsPanel,
    /// The connection in use, loaded from the suite's store.
    connection: Option<ai::Connection>,
    stream: Option<ai::Stream>,
    /// A model list fetched from the provider, waiting for the form.
    models_job: Option<std::sync::mpsc::Receiver<Result<Vec<String>, String>>>,
}

impl Default for Chat {
    fn default() -> Self {
        let mut panel = ui::ChatPanel::default();
        panel.set_policy(ui::WindowPolicy { turns: ui::Turns::LastN(8), total_budget_chars: BUDGET_CHARS });
        Self {
            open: false,
            settings_open: false,
            panel,
            strip: ui::StatusStrip::default(),
            connections: ui::ConnectionsPanel::default(),
            connection: None,
            stream: None,
            models_job: None,
        }
    }
}

impl Chat {
    /// Loads the saved connection, if there is one.
    pub fn load(&mut self) {
        match ai::connections() {
            Ok(all) => self.connection = all.into_iter().next(),
            Err(error) => log::warn!("ai: cannot read the connections: {error}"),
        }
    }

    /// Whether the panel has somewhere to send a question.
    pub fn ready(&self) -> bool {
        self.connection.as_ref().is_some_and(|conn| !conn.model.trim().is_empty())
    }

    pub fn model(&self) -> &str {
        self.connection.as_ref().map(|conn| conn.model.as_str()).unwrap_or_default()
    }

    /// The connection in use, for the other AI actions of the app (the flashcards action sends its
    /// own prompts and must not duplicate the settings).
    pub fn connection(&self) -> Option<&ai::Connection> {
        self.connection.as_ref()
    }

    /// Draws the panel beside the note.
    pub fn ui(&mut self, ui: &mut egui::Ui, state: &mut VaultState) {
        // The app's side of the contract: what the panel is, and what goes as context.
        self.panel.set_system(system_prompt());
        self.panel.set_context(self.context_of(state));
        let model = self.model().to_string();
        let labels = chat_labels(&model, self.ready());
        let strip_action = self.strip.show(ui, ui::StripLabels { stop: tl!("Stop"), dismiss: tl!("Close") });
        if strip_action == ui::StripAction::Stop {
            self.stop();
        }
        ui.separator();
        match self.panel.show(ui, &labels) {
            ui::ChatAction::None => {}
            ui::ChatAction::Send | ui::ChatAction::Retry => self.start(),
            ui::ChatAction::Stop => self.stop(),
            ui::ChatAction::Clear => self.panel.clear(),
            ui::ChatAction::Insert => {
                self.insert_answer(state);
            }
            ui::ChatAction::Settings => self.settings_open = true,
            ui::ChatAction::Close => self.close(),
        }
    }

    /// The note as the panel's context: the tail of the buffer, within the budget.
    fn context_of(&self, state: &VaultState) -> Option<ui::ContextSource> {
        state.active.as_ref()?;
        Some(ui::ContextSource::new(ui::ContextKind::Note, tl!("the note"), state.buffer.clone(), CONTEXT_LIMIT, ui::Side::Tail))
    }

    /// Hands the panel's request to the shared client.
    fn start(&mut self) {
        let Some(lines) = self.panel.take_request() else { return };
        let Some(connection) = self.connection.clone() else {
            self.panel.finish(Err(tl!("configure the connection first").to_string()));
            return;
        };
        let messages: Vec<ai::Msg> = lines
            .into_iter()
            .map(|line| match line.role {
                ui::Role::System => ai::Msg::System(line.text),
                ui::Role::User => ai::Msg::User(line.text),
                ui::Role::Assistant => ai::Msg::Assistant(line.text),
            })
            .collect();
        self.strip.begin();
        self.stream = Some(ai::Stream::start(connection, messages));
    }

    /// Stops a streaming answer; the panel keeps what arrived.
    fn stop(&mut self) {
        if let Some(stream) = &self.stream {
            stream.cancel();
        }
        self.stream = None;
        self.strip.finish();
        self.panel.stopped();
    }

    /// What arrived since the last frame. Called once per frame from the app's own loop.
    pub fn poll(&mut self) {
        if let Some(job) = &self.models_job {
            match job.try_recv() {
                Ok(Ok(models)) => {
                    self.connections.models = models;
                    self.models_job = None;
                }
                Ok(Err(error)) => {
                    self.connections.failed(error);
                    self.models_job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.models_job = None,
            }
        }
        let Some(stream) = &self.stream else { return };
        let (pieces, done) = stream.poll();
        for piece in pieces {
            self.strip.push(&piece);
            self.panel.push(&piece);
        }
        if let Some(result) = done {
            self.strip.finish();
            self.panel.finish(result);
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
        let Some(answer) = self.panel.last_answer().map(str::to_string) else { return false };
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

    /// The panel itself, for the app's tests (the crate owns the transcript).
    #[cfg(test)]
    pub(crate) fn panel_mut(&mut self) -> &mut ui::ChatPanel {
        &mut self.panel
    }

    /// The connections form: the suite's dialog, with this app's store and keychain behind it.
    /// The connections form: the suite's dialog, with this app's store and keychain behind it.
    pub fn settings(&mut self, ui: &mut egui::Ui) -> Option<egui::Rect> {
        if !self.settings_open {
            self.connections.open = false;
            return None;
        }
        if !self.connections.open {
            self.connections.set_presets(presets());
            self.connections.open_with(self.connection.as_ref().map(draft_of));
        }
        let labels = form_labels();
        match self.connections.show(ui.ctx(), &labels) {
            ui::ConnectionsIntent::None => {}
            ui::ConnectionsIntent::Cancel => {
                self.connections.open = false;
                self.settings_open = false;
            }
            ui::ConnectionsIntent::Save(draft, key) => self.save(draft, key),
            ui::ConnectionsIntent::LoadModels(draft, key) => self.fetch_models(draft, key),
        }
        self.connections.rect()
    }

    /// Stores the connection (key in the keychain) and keeps it in use.
    fn save(&mut self, draft: ui::ConnectionDraft, key: String) {
        let connection = connection_of(&draft);
        if let Err(error) = connection.set_key(&ai::APP, &key) {
            log::warn!("ai: cannot store the key: {error}");
            self.connections.failed(error);
            return;
        }
        match ai::save(std::slice::from_ref(&connection)) {
            Ok(()) => {
                self.connection = Some(connection);
                self.panel.clear();
                self.connections.saved();
                self.settings_open = false;
            }
            Err(error) => self.connections.failed(error),
        }
    }

    /// Asks the provider for its model list, on a worker thread.
    fn fetch_models(&mut self, draft: ui::ConnectionDraft, key: String) {
        let connection = connection_of(&draft);
        // The key is not saved yet: the shared client reads it from the keychain, so trial it with
        // the value in the field by storing it first.
        if let Err(error) = connection.set_key(&ai::APP, &key) {
            self.connections.failed(error);
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("ai-models".into())
            .spawn(move || {
                let _ = sender.send(ai::models(&connection));
            })
            .map_err(|error| log::warn!("ai: cannot start the models thread: {error}"))
            .ok();
        self.models_job = Some(receiver);
    }
}

/// The connection a draft describes (one connection per app, so the id is fixed).
fn connection_of(draft: &ui::ConnectionDraft) -> ai::Connection {
    ai::Connection {
        id: "default".to_string(),
        provider: draft.provider.trim().to_string(),
        label: "Local Obsidian".to_string(),
        base_url: draft.base_url.trim().to_string(),
        model: draft.model.trim().to_string(),
    }
}

/// The draft the form shows for the connection in use.
fn draft_of(connection: &ai::Connection) -> ui::ConnectionDraft {
    ui::ConnectionDraft { provider: connection.provider.clone(), base_url: connection.base_url.clone(), model: connection.model.clone() }
}

/// The provider presets the form offers, from the shared client's table.
fn presets() -> Vec<ui::Preset> {
    ai::PROVIDERS
        .iter()
        .map(|provider| ui::Preset { id: provider.id.to_string(), label: provider.label.to_string(), base_url: provider.base_url.to_string() })
        .collect()
}

/// The app's system prompt: the vocabulary of this app, and how the answer must look.
fn system_prompt() -> String {
    tl!("You are a writing assistant inside a Markdown notes app. Answer in the language of the question, in plain Markdown, without preamble.")
        .to_string()
}

/// The panel's words, in the app's language.
fn chat_labels<'a>(model: &'a str, ready: bool) -> ui::ChatLabels<'a> {
    ui::ChatLabels {
        title: tl!("AI"),
        not_configured: tl!("not configured"),
        configure: tl!("configure the connection first"),
        settings: tl!("Settings"),
        clear: tl!("Clear"),
        close: tl!("Close"),
        send: tl!("Send"),
        stop: tl!("Stop"),
        insert: tl!("Insert into the note"),
        retry: tl!("Retry"),
        dismiss: tl!("Close"),
        empty: tl!("Ask about this note, or about anything."),
        input_hint: if ready { tl!("Ask about this note...") } else { tl!("configure the connection first") },
        chip: &chip,
        model,
        suggestions: &[],
        to_end: tl!("to the end"),
    }
}

/// The context chip: what the last request really sent.
fn chip(info: &ui::ContextInfo) -> String {
    let mut text = i18n::tr_fmt("{label}: {count} characters", &[("label", &info.label), ("count", &info.sent.to_string())]);
    if info.truncated() {
        text.push_str(&i18n::tr_fmt(" (cut from {count})", &[("count", &info.total.to_string())]));
    }
    if info.turns > 0 {
        text.push_str(&i18n::tr_fmt(" · {count} turns", &[("count", &info.turns.to_string())]));
    }
    text
}

/// The connections form's words.
fn form_labels<'a>() -> ui::ConnectionsLabels<'a> {
    ui::ConnectionsLabels {
        title: tl!("AI connection"),
        provider: tl!("Provider"),
        base_url: tl!("Base URL"),
        model: tl!("Model"),
        api_key: tl!("API key"),
        list_models: tl!("List models"),
        save: tl!("Save"),
        cancel: tl!("Cancel"),
        hint_url: tl!("Ollama and llama-server are a base URL and a model."),
        hint_key: tl!("Stored in the system keychain, never in a file."),
        close: tl!("Close"),
        needs_key: tl!("Paste the API key first: this provider asks for one."),
        needs_url: tl!("Choose a provider or type its address first."),
        needs_model: tl!("Choose a model first (List models shows them)."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A vault of one note on disk, removed when the test ends.
    fn scratch(tag: &str, notes: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("localobsidian-ai-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch dir");
        for (name, text) in notes {
            std::fs::write(root.join(name), text).expect("write note");
        }
        root
    }

    #[test]
    fn the_answer_is_written_at_the_end_of_the_note() {
        let root = scratch("insert", &[("Nota.md", "# Nota\n\ncorpo\n")]);
        let mut state = VaultState::default();
        state.open(root.clone()).expect("open the vault");
        state.open_note(&root.join("Nota.md"));
        let mut chat = Chat::default();
        chat.panel.push_answer("resposta da IA");
        assert!(chat.insert_answer(&mut state));
        assert!(state.buffer.ends_with("resposta da IA\n"), "{:?}", state.buffer);
        assert_eq!(state.view, crate::vault_state::ViewMode::Split);
        // And the file follows the buffer once it is saved.
        assert!(chat.insert_answer(&mut state), "a second insert appends again");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_answer_without_an_open_note_is_not_written() {
        let root = scratch("none", &[("Nota.md", "# Nota\n")]);
        let mut state = VaultState::default();
        state.open(root.clone()).expect("open the vault");
        let mut chat = Chat::default();
        chat.panel.push_answer("resposta");
        assert!(!chat.insert_answer(&mut state), "no note open, nowhere to write");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_chat_with_no_connection_is_not_ready_and_says_so() {
        let chat = Chat::default();
        assert!(!chat.ready());
        assert!(chat.model().is_empty());
        assert!(chat.connection().is_none());
    }

    #[test]
    fn the_context_is_the_open_note_and_nothing_else() {
        let root = scratch("context", &[("Nota.md", "# Nota\n\ncorpo\n")]);
        let mut state = VaultState::default();
        state.open(root.clone()).expect("open the vault");
        let chat = Chat::default();
        assert!(chat.context_of(&state).is_none(), "no note, no context");
        state.open_note(&root.join("Nota.md"));
        let context = chat.context_of(&state).expect("the open note");
        assert_eq!(context.kind, ui::ContextKind::Note);
        assert_eq!(context.text, state.buffer);
        assert_eq!(context.side, ui::Side::Tail, "the reader is usually at the end");
        let _ = std::fs::remove_dir_all(&root);
    }
}
