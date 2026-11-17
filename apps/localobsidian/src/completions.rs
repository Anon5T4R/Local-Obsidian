//! The `[[wikilink` autocomplete: the popup that appears while a link is being typed.
//!
//! The candidates are the vault's notes whose name *or* alias contains what was typed, ordered the
//! way the link resolver thinks: a name that starts with the text first, then alphabetical - so the
//! popup and the resolver rarely disagree about what `[[sep` means.
//!
//! The list is drawn at the caret, and the keys are handled *before* the text widget draws (see
//! [`crate::editor_keys`]): `Up`/`Down` walk it, `Enter`/`Tab` accept, `Escape` dismisses, and every
//! one of them is consumed so the note does not gain a newline or a tab behind the popup.

use egui::{Key, RichText};

use localobsidian_editor as editor;

use crate::vault_state::VaultState;

/// How many candidates the popup shows.
const MAX_CANDIDATES: usize = 8;

/// A link being typed, and the notes that could complete it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// Where the `[[typed` starts and ends in the note.
    pub range: editor::Selection,
    /// What was typed after `[[`.
    pub typed: String,
    pub candidates: Vec<String>,
    /// The candidate `Up`/`Down` is on.
    pub index: usize,
}

impl Completion {
    /// The candidate that `Enter` would take.
    pub fn selected(&self) -> Option<&str> {
        self.candidates.get(self.index).map(String::as_str)
    }
}

/// The completion for the caret's position, if a link is open there and a note can finish it.
pub fn for_caret(state: &VaultState, buffer: &str, caret: usize) -> Option<Completion> {
    let (range, typed) = editor::wikilink_at(buffer, caret)?;
    // A `|` already typed means the alias is being written: complete the target, keep the alias.
    let target = typed.split('|').next().unwrap_or(&typed).trim();
    let candidates = candidates(state, target);
    if candidates.is_empty() {
        return None;
    }
    Some(Completion { range, typed, candidates, index: 0 })
}

/// The notes that could complete `typed`, best first.
fn candidates(state: &VaultState, typed: &str) -> Vec<String> {
    let needle = typed.to_lowercase();
    let mut names: Vec<(bool, String)> = Vec::new();
    for file in &state.files {
        let info = state.index.info_of(&file.path);
        let name_hit = file.name.to_lowercase().contains(&needle);
        let alias_hit = info.aliases.iter().any(|alias| alias.to_lowercase().contains(&needle));
        if !name_hit && !alias_hit {
            continue;
        }
        let starts = file.name.to_lowercase().starts_with(&needle);
        names.push((!starts, file.name.clone()));
    }
    names.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    names.dedup_by(|a, b| a.1 == b.1);
    names.into_iter().take(MAX_CANDIDATES).map(|(_, name)| name).collect()
}

/// The popup for the next frame, keeping the highlight the reader moved when the same link is still
/// being typed.
///
/// The editor rebuilds the completion from the text on every frame; a rebuilt list without this
/// would start at the first candidate again, so `Down` would never move anywhere.
pub fn refreshed(previous: Option<Completion>, fresh: Option<Completion>) -> Option<Completion> {
    match (previous, fresh) {
        (Some(previous), Some(mut fresh)) if previous.range.start == fresh.range.start => {
            fresh.index = previous.index.min(fresh.candidates.len().saturating_sub(1));
            Some(fresh)
        }
        (_, fresh) => fresh,
    }
}

/// Moves the highlight by `delta`, wrapping around the list.
pub fn move_by(completion: &mut Completion, delta: isize) {
    let len = completion.candidates.len();
    if len == 0 {
        return;
    }
    let index = completion.index as isize + delta;
    completion.index = index.rem_euclid(len as isize) as usize;
}

/// The note text after taking `name` from the popup, and where the caret goes.
///
/// `[[Nota|algo` keeps the alias the author was writing; `[[Nota` becomes `[[Nota]]`.
pub fn accept(buffer: &str, completion: &Completion, name: &str) -> (String, usize) {
    let range = completion.range.clamped(buffer);
    let alias = completion.typed.split_once('|').map(|(_, alias)| alias).unwrap_or_default();
    let replacement = if alias.is_empty() { format!("[[{name}]]") } else { format!("[[{name}|{alias}]]") };
    let mut out = String::with_capacity(buffer.len() + replacement.len());
    out.push_str(&buffer[..range.start]);
    out.push_str(&replacement);
    out.push_str(&buffer[range.end..]);
    let caret = range.start + replacement.len();
    (out, caret)
}

/// Draws the popup at `at`. Returns the candidate the user clicked.
pub fn show(ui: &egui::Ui, completion: &Completion, at: egui::Pos2) -> Option<usize> {
    let mut clicked = None;
    egui::Area::new(ui.scope_id().with("wikilink-completion")).order(egui::Order::Foreground).fixed_pos(at).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_max_width(320.0);
            for (index, name) in completion.candidates.iter().enumerate() {
                let selected = index == completion.index;
                let label = if selected { RichText::new(name).strong() } else { RichText::new(name) };
                if ui.selectable_label(selected, label).clicked() {
                    clicked = Some(index);
                }
            }
        });
    });
    clicked
}

/// The keys the popup answers. Consumed here so the widget never sees them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKey {
    Up,
    Down,
    Accept,
    Dismiss,
}

/// Reads (and consumes) the popup's keys for this frame.
pub fn take_keys(ui: &mut egui::Ui) -> Option<PopupKey> {
    let read = |ui: &egui::Ui, key: Key| ui.input(|input| input.key_pressed(key));
    let picked = if read(ui, Key::ArrowDown) {
        Some((PopupKey::Down, Key::ArrowDown, egui::Modifiers::NONE))
    } else if read(ui, Key::ArrowUp) {
        Some((PopupKey::Up, Key::ArrowUp, egui::Modifiers::NONE))
    } else if read(ui, Key::Tab) {
        Some((PopupKey::Accept, Key::Tab, egui::Modifiers::NONE))
    } else if read(ui, Key::Enter) {
        Some((PopupKey::Accept, Key::Enter, egui::Modifiers::NONE))
    } else if read(ui, Key::Escape) {
        Some((PopupKey::Dismiss, Key::Escape, egui::Modifiers::NONE))
    } else {
        None
    }?;
    ui.input_mut(|input| input.consume_key(picked.2, picked.1));
    Some(picked.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vault of three notes, one of them reachable only by alias.
    fn state() -> VaultState {
        let mut state = VaultState::default();
        let notes =
            [("Sepse", "# Sepse\n"), ("Sepse neonatal", "# Sepse neonatal\n"), ("Protocolo", "---\naliases: [Diagnóstico neonatal]\n---\ncorpo")];
        state.files = notes
            .iter()
            .map(|(name, _)| localobsidian_vault::NoteFile {
                name: (*name).to_string(),
                path: std::path::PathBuf::from(format!("/v/{name}.md")),
                relative_path: format!("{name}.md"),
                mtime_ms: 0,
            })
            .collect();
        let contents: std::collections::BTreeMap<std::path::PathBuf, String> =
            notes.iter().map(|(name, text)| (std::path::PathBuf::from(format!("/v/{name}.md")), (*text).to_string())).collect();
        state.index = localobsidian_vault::index::build(&state.files, &contents);
        state
    }

    fn complete(text: &str) -> Option<Completion> {
        for_caret(&state(), text, text.len())
    }

    #[test]
    fn a_half_typed_link_lists_the_notes_that_fit() {
        let completion = complete("veja [[Sep").expect("a completion");
        assert_eq!(completion.typed, "Sep");
        assert_eq!(completion.candidates, ["Sepse", "Sepse neonatal"]);
    }

    #[test]
    fn a_note_reachable_only_by_alias_is_offered() {
        let completion = complete("[[neon").expect("a completion");
        assert!(completion.candidates.contains(&"Protocolo".to_string()), "{:?}", completion.candidates);
        assert_eq!(completion.candidates.first().map(String::as_str), Some("Protocolo"));
    }

    #[test]
    fn entering_the_name_closes_the_link() {
        let completion = complete("veja [[Sep").expect("a completion");
        let (text, caret) = accept("veja [[Sep", &completion, "Sepse");
        assert_eq!(text, "veja [[Sepse]]");
        assert_eq!(caret, text.len());
    }

    #[test]
    fn an_alias_being_typed_is_kept() {
        let completion = complete("[[Sep|a sepse").expect("a completion");
        let (text, _) = accept("[[Sep|a sepse", &completion, "Sepse");
        assert_eq!(text, "[[Sepse|a sepse]]");
    }

    #[test]
    fn nothing_is_offered_once_the_link_is_closed() {
        assert!(complete("[[Sepse]]").is_none());
    }

    #[test]
    fn the_rebuilt_popup_keeps_the_highlight() {
        // The editor rebuilds the completion every frame from the text; the highlight walks with
        // `Down`, so the rebuild must carry it.
        let mut walked = complete("[[Sep").expect("a completion");
        move_by(&mut walked, 1);
        let fresh = refreshed(Some(walked), complete("[[Sep")).expect("still open");
        assert_eq!(fresh.index, 1, "the highlight survives the rebuild");

        // A link somewhere else in the note starts at the first candidate again.
        let mut other = complete("[[Sep").expect("a completion");
        move_by(&mut other, 1);
        let elsewhere = complete("veja [[Sep");
        assert!(elsewhere.is_some());
        assert_eq!(refreshed(Some(other), elsewhere).map(|completion| completion.index), Some(0));
    }

    #[test]
    fn the_highlight_wraps_around() {
        let mut completion = complete("[[Sep").expect("a completion");
        assert_eq!(completion.index, 0);
        move_by(&mut completion, 1);
        assert_eq!(completion.selected(), Some("Sepse neonatal"));
        move_by(&mut completion, 1);
        assert_eq!(completion.selected(), Some("Sepse"));
        move_by(&mut completion, -1);
        assert_eq!(completion.selected(), Some("Sepse neonatal"));
    }
}
