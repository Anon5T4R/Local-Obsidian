//! AI flashcards: the note in front of the reader becomes a new note of cards beside it.
//!
//! The Ação-Artefato pattern of the suite, with zero destructiveness: the action never writes the
//! note it was started on. It copies the note's structure (frontmatter, tags, title) into a **new**
//! note named `<note> AI`, writes the generated `> [!card]` blocks there, and leaves the original
//! byte for byte as it was - the reader compares the two and keeps what is useful. The undo of the
//! whole action is deleting the note it created.
//!
//! The model answers with a JSON contract (`{"cards":[{"question","answer"}]}`); the shared client
//! streams it, the extractor is tolerant about fences and prose, and an answer without cards comes
//! back as an error the dialog shows instead of a note with nothing in it.

use std::path::{Path, PathBuf};
use std::time::Instant;

use localnotebook_ai as ai;
use localnotebook_md::frontmatter;

use crate::i18n;
use crate::tl;
use crate::vault_state::VaultState;

/// How much of the note goes to the model. A book-length note would cost too much and tell the model
/// nothing new; the head is where a note usually says what it is about.
const CONTEXT_LIMIT: usize = 24_000;

/// The cards-per-section choices the dialog offers.
const DEPTHS: [u8; 4] = [1, 2, 3, 5];

/// The tag every generated note carries.
pub const TAG: &str = "flashcards-ai";

/// The language the cards are written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    /// The interface language.
    App,
    English,
    Portuguese,
    Spanish,
    Japanese,
}

impl Language {
    /// The language as the prompt names it (models know the English names best).
    fn prompt_name(self) -> &'static str {
        match self {
            Language::App => match i18n::code() {
                "en" => "English",
                "es" => "Spanish",
                "ja" => "Japanese",
                _ => "Portuguese (Brazil)",
            },
            Language::English => "English",
            Language::Portuguese => "Portuguese (Brazil)",
            Language::Spanish => "Spanish",
            Language::Japanese => "Japanese",
        }
    }

    fn label(self) -> String {
        match self {
            Language::App => tl!("the app language").to_string(),
            // The language names stay in their own names' Latin form: the dialog is read by someone
            // who recognizes "Español" or "Japanese" regardless of the interface language.
            Language::English => "English".to_string(),
            Language::Portuguese => "Português".to_string(),
            Language::Spanish => "Español".to_string(),
            Language::Japanese => "Japanese".to_string(),
        }
    }
}

/// What the dialog is doing.
enum Status {
    /// Parameters on screen, waiting for "Create".
    Idle,
    /// The model is answering.
    Asking(Box<ai::Stream>),
    /// The note was written (or the last attempt failed).
    Answered,
}

/// The action's state, drawn as a dialog and driven from the AI menu, the note's context menu and
/// the keyboard.
pub struct Flashcards {
    pub open: bool,
    /// The note the action was started on.
    origin: Option<PathBuf>,
    /// Its text, frozen when the action started: the model must see the note the reader saw.
    source: String,
    name: String,
    folder: PathBuf,
    depth: u8,
    language: Language,
    status: Status,
    partial: String,
    started: Option<Instant>,
    error: Option<String>,
    /// The note this action created, for the one undo.
    created: Option<PathBuf>,
}

impl Default for Flashcards {
    fn default() -> Self {
        Self {
            open: false,
            origin: None,
            source: String::new(),
            name: String::new(),
            folder: PathBuf::new(),
            depth: 2,
            language: Language::App,
            status: Status::Idle,
            partial: String::new(),
            started: None,
            error: None,
            created: None,
        }
    }
}

impl Flashcards {
    /// Starts the action on the open note (or on `path`, from the tree's own menu).
    pub fn start(&mut self, state: &VaultState, path: Option<&Path>) {
        let Some(note) = path.or(state.active.as_deref()).and_then(|path| state.files.iter().find(|note| note.path == path)) else {
            return;
        };
        let Ok(text) = localnotebook_vault::read_note(&note.path) else {
            return;
        };
        self.origin = Some(note.path.clone());
        self.source = text;
        self.name = note.name.clone();
        self.folder = note.path.parent().map(Path::to_path_buf).unwrap_or_default();
        self.status = Status::Idle;
        self.partial.clear();
        self.started = None;
        self.error = None;
        self.created = None;
        self.open = true;
    }

    /// The name the new note will have, and whether that name is free in the folder.
    fn target_name(&self) -> String {
        free_name(&self.folder, &self.name)
    }

    /// Draws the dialog. Returns nothing: the work (creating, deleting) happens here.
    pub fn dialog(&mut self, ui: &mut egui::Ui, state: &mut VaultState, chat: &crate::ai_ui::Chat) {
        if !self.open {
            return;
        }
        // The answer arrives on a worker thread; polling it here keeps the dialog alive while it
        // streams and lets the strip show the tail.
        if let Status::Asking(stream) = &self.status {
            let (pieces, done) = stream.poll();
            for piece in pieces {
                self.partial.push_str(&piece);
            }
            if let Some(result) = done {
                match result {
                    Ok(answer) => self.apply(state, &answer),
                    Err(error) => {
                        self.error = Some(error);
                        self.status = Status::Answered;
                    }
                }
            }
        }

        let mut open = self.open;
        let mut create = false;
        let mut cancel = false;
        let mut stop = false;
        let asking = matches!(self.status, Status::Asking(_));
        let target = self.target_name();
        let ready = chat.ready();

        egui::Window::new(tl!("Create flashcards from this note"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .min_width(crate::ai_ui::DIALOG_WIDTH)
            .max_width(crate::ai_ui::DIALOG_WIDTH)
            .show(ui.ctx(), |ui| {
                if asking {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        let seconds = self.started.map(|at| at.elapsed().as_secs()).unwrap_or(0);
                        ui.label(i18n::tr_fmt("asking the model… {seconds}s", &[("seconds", &seconds.to_string())]));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(tl!("Stop")).clicked() {
                                stop = true;
                            }
                        });
                    });
                    // The tail of the stream: proof it is alive without a wall of JSON.
                    let tail: String = {
                        let text: String = self.partial.chars().rev().take(240).collect::<Vec<_>>().into_iter().rev().collect();
                        text.rsplit('\n').next().unwrap_or_default().to_string()
                    };
                    ui.label(egui::RichText::new(tail).small().weak());
                    return;
                }

                if let Some(error) = &self.error {
                    ui.label(egui::RichText::new(error).small().color(ui.visuals().warn_fg_color));
                    ui.add_space(4.0);
                }

                // Destructividade zero, said in the dialog itself: a new note, the original untouched.
                ui.label(i18n::tr_fmt("A new note “{name}” will be created beside this one. The original is not changed.", &[("name", &target)]));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tl!("Cards per section")).small().weak());
                    for depth in DEPTHS {
                        if ui.selectable_label(self.depth == depth, depth.to_string()).clicked() {
                            self.depth = depth;
                        }
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tl!("Language")).small().weak());
                    for language in [Language::App, Language::English, Language::Portuguese, Language::Spanish, Language::Japanese] {
                        if ui.selectable_label(self.language == language, language.label()).clicked() {
                            self.language = language;
                        }
                    }
                });
                if !ready {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(tl!("Not connected to a model - configure it in AI > Connection."))
                            .small()
                            .color(ui.visuals().warn_fg_color),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(ready, egui::Button::new(tl!("Create"))).clicked() {
                        create = true;
                    }
                    if ui.button(tl!("Cancel")).clicked() {
                        cancel = true;
                    }
                    if let Some(created) = &self.created {
                        ui.separator();
                        ui.label(egui::RichText::new(i18n::tr_fmt("Created “{name}”", &[("name", &file_name(created))])).small());
                        if ui.small_button(tl!("Undo")).clicked() {
                            self.undo(state);
                        }
                    }
                });
            });
        if stop {
            if let Status::Asking(stream) = &self.status {
                stream.cancel();
            }
            self.error = Some(tl!("stopped before the model finished").to_string());
            self.status = Status::Answered;
        }
        if create && ready {
            self.ask(chat);
        }
        if cancel {
            self.open = false;
        }
        self.open = open;
    }

    /// Starts the streaming request.
    fn ask(&mut self, chat: &crate::ai_ui::Chat) {
        let Some(connection) = chat.connection().cloned() else {
            self.error = Some(tl!("Not connected to a model - configure it in AI > Connection.").to_string());
            return;
        };
        let history = self.prompt();
        self.partial.clear();
        self.error = None;
        self.created = None;
        self.started = Some(Instant::now());
        self.status = Status::Asking(Box::new(ai::Stream::start(connection, history)));
    }

    /// The system + user messages the action sends.
    fn prompt(&self) -> Vec<ai::Msg> {
        let context: String = self.source.chars().take(CONTEXT_LIMIT).collect();
        let user = i18n::tr_fmt(
            "In {language}, write about {count} flashcards per section of the note below. Reply with JSON only.",
            &[("language", self.language.prompt_name()), ("count", &self.depth.to_string())],
        );
        // The JSON schema is the same in every language, so it stays out of the translated sentence.
        let schema = r#"{"cards":[{"question":"...","answer":"..."}]}"#;
        let system = format!(
            "{} {schema}. {}",
            tl!("You write spaced-repetition flashcards for a study vault. Answer with JSON only, no prose:"),
            tl!("The question is short and specific, the answer is one sentence, and nothing that is not in the note is invented.")
        );
        vec![ai::Msg::System(system), ai::Msg::User(format!("{user}\n\n# {}\n\n{context}", self.name))]
    }

    /// Writes the cards into a new note beside the original.
    ///
    /// Public inside the crate so the test can hand it a canned answer: this is the step that runs
    /// after the stream, and the note-writing contract is what matters (the model is not in it).
    pub(crate) fn apply(&mut self, state: &mut VaultState, answer: &str) {
        let cards = ai::cards_from_answer(answer);
        self.status = Status::Answered;
        if cards.is_empty() {
            self.error = Some(tl!("The model answered without cards. Try again.").to_string());
            return;
        }
        let Some(origin) = self.origin.clone() else {
            self.error = Some(tl!("The note this action started on is gone.").to_string());
            return;
        };
        let _ = origin;
        let name = self.name.clone();
        let text = build_note(&self.source, &name, &self.target_name(), &cards);
        let stem = self.target_name();
        match state.create_note_with(&stem, &text, Some(&self.folder)) {
            Ok(path) => {
                self.created = Some(path.clone());
                state.view = crate::vault_state::ViewMode::Read;
                state.message =
                    Some(i18n::tr_fmt("Created “{name}” with {count} cards", &[("name", &file_name(&path)), ("count", &cards.len().to_string())]));
            }
            Err(error) => self.error = Some(i18n::tr_fmt("could not create the note: {error}", &[("error", &error.to_string())])),
        }
    }

    /// The one undo of the action: the note it created is deleted, and nothing else.
    pub fn undo(&mut self, state: &mut VaultState) {
        let Some(path) = self.created.take() else { return };
        match std::fs::remove_file(&path) {
            Ok(()) => {
                if state.active.as_deref() == Some(path.as_path()) {
                    state.active = None;
                }
                let _ = state.reload();
                state.message = Some(i18n::tr_fmt("Removed “{name}”", &[("name", &file_name(&path))]));
            }
            Err(error) => {
                self.created = Some(path);
                self.error = Some(i18n::tr_fmt("could not remove the note: {error}", &[("error", &error.to_string())]));
            }
        }
    }
}

/// The note name the action writes: `<note> AI`, and `… AI 2` when that one is taken.
fn free_name(folder: &Path, name: &str) -> String {
    for attempt in 1..20 {
        let candidate = if attempt == 1 { format!("{name} AI") } else { format!("{name} AI {attempt}") };
        if !folder.join(format!("{candidate}.md")).exists() {
            return candidate;
        }
    }
    // Twenty notes of the same name in one folder: the clock is still a name nobody has.
    let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_millis()).unwrap_or_default();
    format!("{name} AI {millis}")
}

/// The file name of a path, for the messages.
fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default()
}

/// The new note's text: the original's frontmatter (with the action's tag) and one `[!card]` per
/// answer, the answer as a cloze term so the review panel reads it as a card.
pub fn build_note(original: &str, origin_name: &str, name: &str, cards: &[(String, String)]) -> String {
    let parsed = frontmatter::split(original);
    let mut fields = parsed.frontmatter.unwrap_or_else(frontmatter::Frontmatter::empty);
    let mut tags = fields.tags();
    if !tags.iter().any(|tag| tag == TAG) {
        tags.push(TAG.to_string());
    }
    fields.set_list("tags", &tags);

    let mut out = fields.to_block();
    out.push_str(&format!("\n# {name}\n\n"));
    out.push_str(&i18n::tr_fmt("Generated from [[{note}]] by the AI — check the facts.", &[("note", origin_name)]));
    out.push_str("\n\n");
    for (question, answer) in cards {
        // A card is one line on each side: a cloze term never crosses a line break.
        let question = question.split_whitespace().collect::<Vec<_>>().join(" ");
        let answer = answer.split_whitespace().collect::<Vec<_>>().join(" ");
        if question.is_empty() || answer.is_empty() {
            continue;
        }
        out.push_str(&format!("> [!card] {question}\n> =={answer}==\n\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_new_note_carries_the_original_frontmatter_plus_the_tag() {
        let original = "---\ntipo: sistema\naliases:\n  - Lacunas\ntags: [uti]\n---\n\n# MOC\n\ncorpo\n";
        let note = build_note(original, "MOC - Medicina", "MOC - Medicina AI", &[("Pergunta".into(), "Resposta".into())]);
        assert!(note.contains("tags: [uti, flashcards-ai]") || note.contains("tags:") && note.contains("flashcards-ai"), "{note}");
        assert!(note.contains("aliases"), "the aliases come along: {note}");
        assert!(note.contains("tipo: sistema"), "{note}");
        assert!(note.contains("# MOC - Medicina AI"), "{note}");
        assert!(note.contains("> [!card] Pergunta\n> ==Resposta=="), "the card is the syntax the review reads: {note}");
        assert!(!note.contains("corpo"), "the body is not copied: {note}");
    }

    #[test]
    fn a_note_without_frontmatter_gets_one_with_the_tag() {
        let note = build_note("# Nota\n\ncorpo\n", "Nota", "Nota AI", &[("P".into(), "R".into())]);
        assert!(note.starts_with("---\n"), "{note}");
        assert!(note.contains("flashcards-ai"), "{note}");
    }

    #[test]
    fn a_multi_line_answer_becomes_one_line() {
        let note = build_note("", "Nota", "Nota AI", &[("P".into(), "linha um\nlinha dois".into())]);
        assert!(note.contains("> ==linha um linha dois=="), "{note}");
    }
}
