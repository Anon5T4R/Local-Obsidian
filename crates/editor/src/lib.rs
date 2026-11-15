//! Local Obsidian's Markdown editing commands, with no UI and no filesystem.
//!
//! The editor surface itself is egui's text widget (IME, bidi and clipboard come with it); what
//! lives here are the edits a Markdown note needs and a generic text box does not have: wrapping the
//! selection in `**`/`*`/`` ` ``/`~~`/`==`, list and quote markers on the touched lines, numbered
//! lists that count, headings that cycle, indentation, list continuation on Enter, task checkboxes
//! flipped by source order, and the half-typed `[[` that autocomplete replaces.
//!
//! Every command is a pure function `(text, selection) -> Edit`, so all of it is unit-tested and the
//! window only has to move the caret.

pub mod inline;
pub mod lines;
pub mod text;

pub use inline::{Wrap, toggle_wrap, wikilink_at};
pub use lines::{
    BULLET, INDENT, NextItem, QUOTE, cycle_heading, indent, list_continuation, next_item, toggle_line_task, toggle_numbered, toggle_prefix,
    toggle_task,
};
pub use text::{Edit, Selection, byte_to_char, char_to_byte, line_end, line_number, line_start, touched_lines};
