//! The keys the editor answers besides typing, and the caret bookkeeping behind them.
//!
//! egui's text widget owns the typing, the selection and the clipboard; this module adds what a
//! Markdown note needs and a generic text box does not: `Ctrl+B`/`I`/`E`/`Shift+E` wrap the
//! selection in `**`/`*`/`` ` ``/`==`, `Tab`/`Shift+Tab` indent, `Ctrl+Enter` flips the task on the
//! caret's line, and pressing Enter at the end of a list item starts the next one.
//!
//! List continuation is done *after* the newline is inserted, never by eating the Enter key: an IME
//! uses Enter to confirm a candidate, and stealing it would break Japanese, Korean and Chinese
//! input - the exact thing the suite's fonts exist for.
//!
//! The commands themselves live in `localobsidian_editor`; this module only moves text between the
//! widget's character offsets and the crate's byte offsets, and puts the caret back.

use egui::Key;
use egui::text::{CCursor, CCursorRange};
use egui::widgets::text_edit::TextEditState;

use localobsidian_editor as editor;
use localobsidian_editor::{Selection, Wrap};

/// What a key press asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorKey {
    /// The buffer changed: the note is dirty.
    Changed,
    /// `Ctrl+S` while the editor has the focus.
    Save,
}

/// Runs the editor's keys for the widget with this `id`. Returns what changed, if anything.
pub fn handle(ui: &mut egui::Ui, buffer: &mut String, id: egui::Id) -> Option<EditorKey> {
    if !ui.memory(|memory| memory.has_focus(id)) {
        return None;
    }

    let mut outcome = None;
    let pressed = |ui: &egui::Ui, key: Key, command: bool, shift: bool| {
        ui.input(|input| input.key_pressed(key) && input.modifiers.command == command && input.modifiers.shift == shift && !input.modifiers.alt)
    };

    // Wraps: Ctrl+B (bold), Ctrl+I (italic), Ctrl+E (code), Ctrl+Shift+E (highlight).
    let wrap = if pressed(ui, Key::B, true, false) {
        Some(Wrap::Bold)
    } else if pressed(ui, Key::I, true, false) {
        Some(Wrap::Italic)
    } else if pressed(ui, Key::E, true, true) {
        Some(Wrap::Highlight)
    } else if pressed(ui, Key::E, true, false) {
        Some(Wrap::Code)
    } else {
        None
    };

    if let Some(wrap) = wrap {
        let selection = selection_of(ui, buffer, id);
        let edit = editor::toggle_wrap(buffer, selection, wrap);
        *buffer = edit.text;
        set_selection(ui, buffer, id, edit.selection);
        return Some(EditorKey::Changed);
    }

    if pressed(ui, Key::S, true, false) {
        outcome = Some(EditorKey::Save);
    }

    // Tab / Shift+Tab indent the touched lines.
    if ui.input(|input| input.key_pressed(Key::Tab)) && !ui.input(|input| input.modifiers.command) {
        let outdent = ui.input(|input| input.modifiers.shift);
        let selection = selection_of(ui, buffer, id);
        let edit = editor::indent(buffer, selection, outdent);
        *buffer = edit.text;
        set_selection(ui, buffer, id, edit.selection);
        // Consume it so the widget does not also insert a tab or move the focus.
        ui.input_mut(|input| input.consume_key(input.modifiers, Key::Tab));
        return Some(EditorKey::Changed);
    }

    // Ctrl+Enter flips the task checkbox on the caret's line.
    if pressed(ui, Key::Enter, true, false) {
        let caret = selection_of(ui, buffer, id).end;
        let line = editor::line_start(buffer, caret);
        let end = editor::line_end(buffer, line);
        if let Some(edit) = editor::toggle_line_task(buffer, line..end) {
            *buffer = edit.text;
            set_selection(ui, buffer, id, edit.selection);
            return Some(EditorKey::Changed);
        }
    }

    if continue_list(ui, buffer, id) {
        return Some(EditorKey::Changed);
    }
    outcome
}

/// After Enter, a caret on a fresh empty line whose previous line is a list item gets the next
/// marker. On an item that was empty, the marker that was left behind is taken away (that is how a
/// list ends).
fn continue_list(ui: &mut egui::Ui, buffer: &mut String, id: egui::Id) -> bool {
    let enter = ui.input(|input| input.key_pressed(Key::Enter) && !input.modifiers.command && !input.modifiers.shift);
    if !enter {
        return false;
    }
    let caret = selection_of(ui, buffer, id).end;
    if caret == 0 || !buffer[..caret].ends_with('\n') {
        return false;
    }
    let previous_start = editor::line_start(buffer, caret.saturating_sub(1));
    let previous_end = caret - 1;
    let previous = &buffer[previous_start..previous_end];

    let Some(item) = editor::next_item(previous) else { return false };
    if item.empty {
        // Enter on an empty item ends the list: the marker that was left behind goes away.
        let marker_start = previous_start + item.lead_len;
        buffer.replace_range(marker_start..previous_end, "");
        set_selection(ui, buffer, id, Selection::caret(caret - (previous_end - marker_start)));
        return true;
    }

    buffer.insert_str(caret, &item.marker);
    set_selection(ui, buffer, id, Selection::caret(caret + item.marker.len()));
    true
}

/// The widget's selection, in bytes.
fn selection_of(ui: &egui::Ui, buffer: &str, id: egui::Id) -> Selection {
    let Some(state) = TextEditState::load(ui.ctx(), id) else { return Selection::caret(buffer.len()) };
    let Some(range) = state.cursor.char_range() else { return Selection::caret(buffer.len()) };
    let sorted = range.as_sorted_char_range();
    let start = editor::char_to_byte(buffer, usize::from(sorted.start));
    let end = editor::char_to_byte(buffer, usize::from(sorted.end));
    Selection::new(start, end)
}

/// Puts the caret (or the selection) back, in the widget's character offsets.
fn set_selection(ui: &egui::Ui, buffer: &str, id: egui::Id, selection: Selection) {
    let Some(mut state) = TextEditState::load(ui.ctx(), id) else { return };
    let start = editor::byte_to_char(buffer, selection.start);
    let end = editor::byte_to_char(buffer, selection.end);
    let range = CCursorRange::two(CCursor::new(start), CCursor::new(end));
    state.cursor.set_char_range(Some(range));
    state.store(ui.ctx(), id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_selection_survives_the_byte_and_char_round_trip() {
        let text = "açúcar com [[Nota]]";
        for byte in [0, 1, 3, 5, 7, 12, 20] {
            let chars = editor::byte_to_char(text, byte);
            assert_eq!(editor::char_to_byte(text, chars), byte, "byte {byte}");
        }
    }
}
