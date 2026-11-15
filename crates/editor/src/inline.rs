//! Inline formatting: the markers a Markdown note puts *around* a run of text.
//!
//! One command, `toggle_wrap`, serves bold, italic, inline code, strikethrough and highlight: the
//! keyboard shortcut decides the marker, and pressing it again takes it off. With nothing selected
//! the markers go in and the caret lands between them, so the next word typed is formatted.

use crate::text::{Edit, Selection, floor_boundary};

/// The markers the editor offers. `Highlight` is Obsidian's `==text==`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    Bold,
    Italic,
    Code,
    Strike,
    Highlight,
}

impl Wrap {
    pub fn marker(self) -> &'static str {
        match self {
            Wrap::Bold => "**",
            Wrap::Italic => "*",
            Wrap::Code => "`",
            Wrap::Strike => "~~",
            Wrap::Highlight => "==",
        }
    }
}

/// Wraps the selection with `wrap`'s marker, or removes it when the text is already wrapped.
pub fn toggle_wrap(text: &str, selection: Selection, wrap: Wrap) -> Edit {
    let marker = wrap.marker();
    let selection = selection.clamped(text);

    if selection.is_empty() {
        let at = selection.start;
        let mut out = String::with_capacity(text.len() + marker.len() * 2);
        out.push_str(&text[..at]);
        out.push_str(marker);
        out.push_str(marker);
        out.push_str(&text[at..]);
        return Edit { text: out, selection: Selection::caret(at + marker.len()) };
    }

    let before = &text[..selection.start];
    let after = &text[selection.end..];
    if before.ends_with(marker) && after.starts_with(marker) {
        // `**palavra**` with the caret inside: take both markers out.
        let start = selection.start - marker.len();
        let mut out = String::with_capacity(text.len() - marker.len() * 2);
        out.push_str(&text[..start]);
        out.push_str(&text[selection.start..selection.end]);
        out.push_str(&text[selection.end + marker.len()..]);
        return Edit { text: out, selection: Selection::new(start, start + (selection.end - selection.start)) };
    }

    let selected = &text[selection.start..selection.end];
    if selected.len() >= marker.len() * 2 && selected.starts_with(marker) && selected.ends_with(marker) {
        // The selection itself carries the markers: strip them and keep the words selected.
        let inner_start = marker.len();
        let inner_end = selected.len() - marker.len();
        let mut out = String::with_capacity(text.len() - marker.len() * 2);
        out.push_str(&text[..selection.start]);
        out.push_str(&selected[inner_start..inner_end]);
        out.push_str(&text[selection.end..]);
        let start = selection.start;
        return Edit { text: out, selection: Selection::new(start, start + (inner_end - inner_start)) };
    }

    let mut out = String::with_capacity(text.len() + marker.len() * 2);
    out.push_str(before);
    out.push_str(marker);
    out.push_str(selected);
    out.push_str(marker);
    out.push_str(after);
    Edit { text: out, selection: Selection::new(selection.start + marker.len(), selection.end + marker.len()) }
}

/// The `[[` a caret is standing inside, when there is one: the range of the brackets and the text
/// typed so far. Autocomplete uses it; `None` once the link is closed or the carets leaves it.
pub fn wikilink_at(text: &str, caret: usize) -> Option<(Selection, String)> {
    let caret = floor_boundary(text, caret);
    let open = text[..caret].rfind("[[")?;
    // A closed link before the caret is not the one being typed.
    let closed = text[open..caret].rfind("]]");
    if closed.is_some() {
        return None;
    }
    let typed = &text[open + 2..caret];
    // A link never spans a line.
    if typed.contains('\n') || typed.contains('[') {
        return None;
    }
    Some((Selection::new(open, caret), typed.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caret(at: usize) -> Selection {
        Selection::caret(at)
    }

    #[test]
    fn wrapping_the_selection_keeps_it_selected_inside_the_markers() {
        let edit = toggle_wrap("uma palavra", Selection::new(4, 11), Wrap::Bold);
        assert_eq!(edit.text, "uma **palavra**");
        assert_eq!(&edit.text[edit.selection.start..edit.selection.end], "palavra");
    }

    #[test]
    fn pressing_it_again_takes_the_markers_off() {
        let edit = toggle_wrap("uma **palavra**", Selection::new(6, 13), Wrap::Bold);
        assert_eq!(edit.text, "uma palavra");
        assert_eq!(&edit.text[edit.selection.start..edit.selection.end], "palavra");
    }

    #[test]
    fn a_selection_that_already_carries_the_markers_is_unwrapped() {
        let edit = toggle_wrap("uma **palavra**", Selection::new(4, 15), Wrap::Bold);
        assert_eq!(edit.text, "uma palavra");
        assert_eq!(&edit.text[edit.selection.start..edit.selection.end], "palavra");
    }

    #[test]
    fn a_caret_gets_the_pair_with_the_caret_between() {
        let edit = toggle_wrap("uma ", caret(4), Wrap::Italic);
        assert_eq!(edit.text, "uma **");
        assert_eq!(edit.selection.start, 5);
        let edit = toggle_wrap("x", caret(1), Wrap::Code);
        assert_eq!(edit.text, "x``");
        assert_eq!(edit.selection.start, 2);
    }

    #[test]
    fn accents_are_never_split() {
        let text = "café com leite";
        let start = text.find("com").unwrap_or(0);
        let edit = toggle_wrap(text, Selection::new(start, start + 3), Wrap::Strike);
        assert_eq!(edit.text, "café ~~com~~ leite");
        assert_eq!(&edit.text[edit.selection.start..edit.selection.end], "com");
    }

    #[test]
    fn a_stale_position_outside_the_text_is_clamped() {
        let edit = toggle_wrap("abc", Selection::new(99, 120), Wrap::Bold);
        // Clamped to the end of the text: an empty selection there gets the pair.
        assert_eq!(edit.text, "abc****");
        assert_eq!(edit.selection, Selection::caret(5));
    }

    #[test]
    fn the_wikilink_being_typed_is_found() {
        let (range, typed) = wikilink_at("veja [[Sep", 11).expect("a link is open");
        assert_eq!(typed, "Sep");
        assert_eq!(&"veja [[Sep"[range.start..range.end], "[[Sep");
    }

    #[test]
    fn a_closed_link_and_a_line_break_end_the_wikilink() {
        assert!(wikilink_at("[[Sepse]] e ", 12).is_none());
        assert!(wikilink_at("[[Sep\nse", 8).is_none());
        assert!(wikilink_at("sem link", 8).is_none());
    }
}
