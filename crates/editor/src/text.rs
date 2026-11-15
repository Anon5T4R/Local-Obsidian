//! Byte and character positions inside a note, and the small helpers every command needs.
//!
//! The editor works in byte offsets (what `String` slicing wants) while egui's text widget counts
//! characters (what its cursor wants), so both directions live here, next to the rule that matters:
//! a position is never inside a character.

/// A range of the note, in bytes, always on character boundaries and with `start <= end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    pub start: usize,
    pub end: usize,
}

impl Selection {
    /// The range between two positions, in whichever order they arrive.
    pub fn new(one: usize, other: usize) -> Self {
        if one <= other { Self { start: one, end: other } } else { Self { start: other, end: one } }
    }

    /// A caret: an empty range at `at`.
    pub fn caret(at: usize) -> Self {
        Self { start: at, end: at }
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// `self`, brought inside `text` and onto character boundaries.
    pub fn clamped(self, text: &str) -> Self {
        let start = floor_boundary(text, self.start.min(text.len()));
        let end = floor_boundary(text, self.end.min(text.len()));
        Self::new(start, end)
    }
}

/// The text a command produced, and where the caret (or selection) goes after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub text: String,
    pub selection: Selection,
}

/// The largest character boundary at or below `index`.
pub fn floor_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// The character index (what egui counts) of a byte offset.
pub fn byte_to_char(text: &str, byte: usize) -> usize {
    text[..floor_boundary(text, byte)].chars().count()
}

/// The byte offset of a character index.
pub fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices().nth(char_index).map(|(byte, _)| byte).unwrap_or(text.len())
}

/// Where the line holding `at` starts.
pub fn line_start(text: &str, at: usize) -> usize {
    let at = floor_boundary(text, at);
    text[..at].rfind('\n').map_or(0, |newline| newline + 1)
}

/// Where the line holding `at` ends: the `\n` is not part of it.
pub fn line_end(text: &str, at: usize) -> usize {
    let at = floor_boundary(text, at);
    text[at..].find('\n').map_or(text.len(), |newline| at + newline)
}

/// The `(start, end)` of every line the selection touches, in order.
pub fn touched_lines(text: &str, selection: Selection) -> Vec<(usize, usize)> {
    let selection = selection.clamped(text);
    let mut out = Vec::new();
    let mut line = line_start(text, selection.start);
    loop {
        let end = line_end(text, line);
        out.push((line, end));
        if end >= selection.end || end >= text.len() {
            break;
        }
        line = end + 1;
    }
    out
}

/// The number of the line holding `at`, counted from zero.
pub fn line_number(text: &str, at: usize) -> usize {
    text[..floor_boundary(text, at)].matches('\n').count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selection_orders_its_ends() {
        assert_eq!(Selection::new(5, 2), Selection { start: 2, end: 5 });
        assert!(Selection::caret(3).is_empty());
    }

    #[test]
    fn a_position_inside_a_character_moves_down_to_its_start() {
        let text = "açúcar";
        // 'ç' is two bytes: the offset between them is not a boundary.
        assert_eq!(floor_boundary(text, 2), 1);
        assert_eq!(floor_boundary(text, 100), text.len());
        assert_eq!(Selection { start: 2, end: 3 }.clamped(text), Selection { start: 1, end: 3 });
    }

    #[test]
    fn chars_and_bytes_convert_both_ways() {
        let text = "açúcar\n第二";
        for (byte, _) in text.char_indices() {
            assert_eq!(char_to_byte(text, byte_to_char(text, byte)), byte);
        }
        assert_eq!(char_to_byte(text, 999), text.len());
    }

    #[test]
    fn lines_are_delimited_by_newlines_without_including_them() {
        let text = "um\ndois\ntrês";
        // `três` has a two-byte `ê`, so the byte positions are not the character positions.
        assert_eq!((line_start(text, 5), line_end(text, 5)), (3, 7));
        assert_eq!(line_number(text, 5), 1);
        assert_eq!(line_start(text, 0), 0);
        assert_eq!(line_end(text, text.len()), text.len());
    }

    #[test]
    fn a_selection_touches_every_line_it_covers() {
        let text = "um\ndois\ntrês";
        assert_eq!(touched_lines(text, Selection::new(1, 6)), vec![(0, 2), (3, 7)]);
        assert_eq!(touched_lines(text, Selection::caret(0)), vec![(0, 2)]);
    }

    #[test]
    fn a_selection_ending_at_a_line_start_does_not_touch_the_next_line() {
        let text = "um\ndois";
        // End right on the `\n` of line one.
        assert_eq!(touched_lines(text, Selection::new(0, 2)), vec![(0, 2)]);
    }
}
