//! Where the reader has been: the back and forward list of a browser, for notes.
//!
//! Four gestures mean the same thing here — the two arrows in the note header, `Alt+←`/`Alt+→`, and
//! the mouse's side buttons (`XButton1`/`XButton2`, which every browser reads as back and forward).
//! [`requested`] turns the raw input of a frame into the one direction it asks for.
//!
//! The list is a pair of stacks around the note that is open: opening a note pushes the previous one
//! onto `back` and clears `ahead` (the future you had walked away from); going back moves the current
//! note onto `ahead`. A note is never pushed twice in a row, and the list stops at [`LIMIT`] entries;
//! a vault is browsed for hours, and the first note of the morning is not worth a megabyte.

use std::path::{Path, PathBuf};

/// How many notes the history remembers.
pub const LIMIT: usize = 100;

/// Which way a gesture asks to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Back,
    Forward,
}

/// The notes already visited, and the ones a `back` walked away from.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct History {
    back: Vec<PathBuf>,
    ahead: Vec<PathBuf>,
}

impl History {
    /// A note was opened from `from`: the note that was open becomes the way back.
    pub fn visit(&mut self, from: Option<&Path>, to: &Path) {
        let Some(from) = from else { return };
        if from == to {
            return;
        }
        self.back.push(from.to_path_buf());
        self.trim();
        // Opening a note by hand is a new branch: the notes ahead are no longer reachable by the
        // reader's own path through the vault.
        self.ahead.clear();
    }

    /// The note back would open, without moving: the caller opens it first, so a note that has
    /// disappeared from the vault does not eat a step of the history.
    pub fn peek_back(&self) -> Option<PathBuf> {
        self.back.last().cloned()
    }

    /// The note forward would open, without moving.
    pub fn peek_forward(&self) -> Option<PathBuf> {
        self.ahead.last().cloned()
    }

    /// Takes the back step: `current` becomes the way forward.
    pub fn step_back(&mut self, current: &Path) -> Option<PathBuf> {
        let target = self.back.pop()?;
        self.ahead.push(current.to_path_buf());
        Some(target)
    }

    /// Takes the forward step: `current` becomes the way back.
    pub fn step_forward(&mut self, current: &Path) -> Option<PathBuf> {
        let target = self.ahead.pop()?;
        self.back.push(current.to_path_buf());
        Some(target)
    }

    /// A note was renamed: the entries that pointed at it point at its new path.
    pub fn retarget(&mut self, from: &Path, to: &Path) {
        for entry in self.back.iter_mut().chain(self.ahead.iter_mut()) {
            if entry == from {
                *entry = to.to_path_buf();
            }
        }
    }

    /// The history starts over (another vault was opened).
    pub fn clear(&mut self) {
        self.back.clear();
        self.ahead.clear();
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.ahead.is_empty()
    }

    /// How many notes the history holds, both ways (the tests' view of the stacks).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.back.len() + self.ahead.len()
    }

    /// Whether the history is empty (a vault that was just opened).
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.back.is_empty() && self.ahead.is_empty()
    }

    fn trim(&mut self) {
        if self.back.len() > LIMIT {
            self.back.remove(0);
        }
    }
}

/// Which way the input of this frame asks to go: `Alt+←`/`Alt+→`, or the mouse's side buttons.
///
/// One gesture per frame: back wins when both arrive, which only happens with two hands on two
/// devices at once.
pub fn requested(ctx: &egui::Context) -> Option<Direction> {
    let (back, forward) = ctx.input(|input| {
        (
            (input.modifiers.alt && input.key_pressed(egui::Key::ArrowLeft)) || input.pointer.button_pressed(egui::PointerButton::Extra1),
            (input.modifiers.alt && input.key_pressed(egui::Key::ArrowRight)) || input.pointer.button_pressed(egui::PointerButton::Extra2),
        )
    });
    if back {
        Some(Direction::Back)
    } else if forward {
        Some(Direction::Forward)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(format!("/v/{name}.md"))
    }

    /// Opens a sequence of notes, the way the app does.
    fn walk(history: &mut History, notes: &[&str]) -> PathBuf {
        let mut current = path(notes[0]);
        for next in &notes[1..] {
            let next = path(next);
            history.visit(Some(&current), &next);
            current = next;
        }
        current
    }

    #[test]
    fn back_walks_the_notes_in_reverse_and_forward_returns() {
        let mut history = History::default();
        assert!(history.is_empty(), "a session starts with nowhere to go");
        let current = walk(&mut history, &["a", "b", "c"]);
        assert_eq!(current, path("c"));
        assert_eq!(history.len(), 2, "two notes behind this one");

        let b = history.step_back(&current).expect("one step back");
        assert_eq!(b, path("b"));
        let a = history.step_back(&b).expect("two steps back");
        assert_eq!(a, path("a"));
        assert!(!history.can_go_back(), "the first note is the end of the line");

        let b = history.step_forward(&a).expect("forward again");
        assert_eq!(b, path("b"));
        let c = history.step_forward(&b).expect("forward to where we were");
        assert_eq!(c, path("c"));
        assert!(!history.can_go_forward());
    }

    #[test]
    fn opening_a_note_by_hand_drops_the_way_forward() {
        let mut history = History::default();
        walk(&mut history, &["a", "b", "c"]);
        let b = history.step_back(&path("c")).expect("back");
        assert!(history.can_go_forward());
        history.visit(Some(&b), &path("d"));
        assert!(!history.can_go_forward(), "a new path through the vault has no future yet");
        assert_eq!(history.step_back(&path("d")), Some(path("b")));
    }

    #[test]
    fn the_same_note_is_not_remembered_twice_in_a_row() {
        let mut history = History::default();
        history.visit(Some(&path("a")), &path("a"));
        assert!(!history.can_go_back(), "reopening the same note is not a step");
        history.visit(Some(&path("a")), &path("b"));
        history.visit(Some(&path("b")), &path("b"));
        assert_eq!(history.len(), 1);
        assert_eq!(history.step_back(&path("b")), Some(path("a")));
    }

    #[test]
    fn the_history_stops_at_a_hundred_notes() {
        let mut history = History::default();
        let mut current = path("0");
        for index in 1..=(LIMIT + 25) {
            let next = path(&index.to_string());
            history.visit(Some(&current), &next);
            current = next;
        }
        assert_eq!(history.back.len(), LIMIT, "the list has a ceiling");
        // The oldest entries went first: the end of the line is the 25th note, not the first.
        let mut step = history.step_back(&current).expect("still has history");
        let mut last = step.clone();
        while let Some(next) = history.step_back(&last) {
            step = next.clone();
            last = next;
        }
        assert_eq!(step, path("25"), "the first notes of the session were let go");
    }

    #[test]
    fn a_rename_moves_the_history_entries_with_the_note() {
        let mut history = History::default();
        walk(&mut history, &["a", "b"]);
        history.retarget(&path("a"), &path("a-renomeada"));
        assert_eq!(history.step_back(&path("b")), Some(path("a-renomeada")));
    }

    #[test]
    fn alt_arrows_and_the_side_buttons_both_ask_for_a_direction() {
        let ctx = egui::Context::default();
        let ask = |events: Vec<egui::Event>| {
            let mut answer = None;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_pass(raw, |ctx| answer = requested(ctx));
            // The pass is not painted here: the texture deltas have to be handled or dropped on
            // purpose, and there is no canvas in a unit test.
            output.textures_delta.clear();
            answer
        };
        let alt = egui::Modifiers { alt: true, ..Default::default() };
        let key = |key: egui::Key, alt: bool| {
            let modifiers = egui::Modifiers { alt, ..Default::default() };
            // The backend reports the modifier state as its own event; the key carries it too.
            vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }]
        };
        let button = |button: egui::PointerButton| egui::Event::PointerButton {
            pos: egui::pos2(10.0, 10.0),
            button,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        };

        ctx.begin_pass(egui::RawInput::default());
        assert_eq!(requested(&ctx), None, "no gesture, no direction");
        let mut empty = ctx.end_pass();
        empty.textures_delta.clear();

        assert_eq!(ask(vec![button(egui::PointerButton::Extra1)]), Some(Direction::Back));
        assert_eq!(ask(vec![button(egui::PointerButton::Extra2)]), Some(Direction::Forward));
        assert_eq!(ask(key(egui::Key::ArrowLeft, true)), Some(Direction::Back));
        assert_eq!(ask(key(egui::Key::ArrowRight, true)), Some(Direction::Forward));
        // An arrow without `Alt` is the caret's: a note is a text box first.
        assert_eq!(ask(key(egui::Key::ArrowLeft, false)), None);
        let _ = alt;
    }
}
