//! The keys the editor answers besides typing, and the caret bookkeeping behind them.
//!
//! egui's text widget owns the typing, the selection and the clipboard; this module adds what a
//! Markdown note needs and a generic text box does not: `Ctrl+B`/`I`/`E`/`Shift+E` wrap the
//! selection in `**`/`*`/`` ` ``/`==`, `Tab`/`Shift+Tab` indent, `Ctrl+Enter` flips the task on the
//! caret's line, and pressing Enter at the end of a list item starts the next one.
//!
//! Two moments, because order matters: [`pre`] runs *before* the widget is added - the keys it
//! answers are consumed there, so the widget never sees them and a `Tab` cannot both indent and
//! insert a tab - and [`post`] runs after, because list continuation reads the newline the widget
//! has just inserted. Continuation is never done by eating Enter itself: an IME uses Enter to
//! confirm a candidate, and stealing it would break Japanese, Korean and Chinese input.
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

/// The command keys, run before the text widget draws. Applies the edit to `buffer` and leaves the
/// caret where the command left it.
pub fn pre(ui: &mut egui::Ui, buffer: &mut String, id: egui::Id) -> Option<EditorKey> {
    if !ui.memory(|memory| memory.has_focus(id)) {
        return None;
    }

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
        let edit = editor::toggle_wrap(buffer, selection_of(ui, buffer, id), wrap);
        apply(ui, buffer, id, edit);
        return Some(EditorKey::Changed);
    }

    // Tab / Shift+Tab indent the touched lines. Consumed here so the widget inserts no tab.
    if ui.input(|input| input.key_pressed(Key::Tab)) && !ui.input(|input| input.modifiers.command) {
        let outdent = ui.input(|input| input.modifiers.shift);
        ui.input_mut(|input| input.consume_key(input.modifiers, Key::Tab));
        let edit = editor::indent(buffer, selection_of(ui, buffer, id), outdent);
        apply(ui, buffer, id, edit);
        return Some(EditorKey::Changed);
    }

    // Ctrl+Enter flips the task checkbox on the caret's line. Consumed: the widget would otherwise
    // insert a newline on top of the flip.
    if pressed(ui, Key::Enter, true, false) {
        ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, Key::Enter));
        let caret = selection_of(ui, buffer, id).end;
        let line = editor::line_start(buffer, caret);
        let end = editor::line_end(buffer, line);
        if let Some(edit) = editor::toggle_line_task(buffer, line..end) {
            apply(ui, buffer, id, edit);
            return Some(EditorKey::Changed);
        }
    }

    if pressed(ui, Key::S, true, false) {
        return Some(EditorKey::Save);
    }
    None
}

/// After the widget ran: a caret on a fresh line whose previous line is a list item gets the next
/// marker. On an item that was empty, the marker left behind is taken away (that is how a list ends).
pub fn post(ui: &mut egui::Ui, buffer: &mut String, id: egui::Id) -> bool {
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
        let marker_start = previous_start + item.lead_len;
        buffer.replace_range(marker_start..previous_end, "");
        set_selection(ui, buffer, id, Selection::caret(caret - (previous_end - marker_start)));
        return true;
    }

    buffer.insert_str(caret, &item.marker);
    set_selection(ui, buffer, id, Selection::caret(caret + item.marker.len()));
    true
}

/// Runs *before* the text widget draws.
fn pressed(ui: &egui::Ui, key: Key, command: bool, shift: bool) -> bool {
    ui.input(|input| input.key_pressed(key) && input.modifiers.command == command && input.modifiers.shift == shift && !input.modifiers.alt)
}

/// Puts the caret at a byte offset (the popup and a search result both need it).
pub fn place_caret(ui: &egui::Ui, buffer: &str, id: egui::Id, byte: usize) {
    set_selection(ui, buffer, id, Selection::caret(byte));
}

/// The caret's byte offset.
pub fn caret_byte(ui: &egui::Ui, buffer: &str, id: egui::Id) -> usize {
    selection_of(ui, buffer, id).end
}

/// Writes the edit into the buffer and restores the caret.
fn apply(ui: &egui::Ui, buffer: &mut String, id: egui::Id, edit: editor::Edit) {
    *buffer = edit.text;
    set_selection(ui, buffer, id, edit.selection);
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
