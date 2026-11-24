//! Find in the note being read: the bar, the matches, and where the reader is.
//!
//! The vault search (`Ctrl+Shift+F`) answers "where in the vault"; this answers "where in this
//! note", scoped to what the reading view draws. A match is highlighted in place and the bar walks
//! them with `Enter`/`Shift+Enter` or the two arrows.

use std::cell::Cell;

/// The find bar's state.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Find {
    pub open: bool,
    pub query: String,
    /// Which match the reader is on, zero-based.
    pub current: usize,
    /// How many matches the note has (from the last drawn frame).
    pub total: usize,
    /// Set when the bar opens: the field takes the keyboard once, and not every frame after.
    pub focus: bool,
    /// The view must bring the current match into sight **once** (the bar opened, the query changed,
    /// the reader walked the matches). Scrolling on every frame would win against the wheel, and a
    /// note being read could not be scrolled while a match is highlighted.
    pub scroll_to_current: Cell<bool>,
    /// The matches drawn so far this frame; the view walks the note once and this keeps count.
    pub seen: Cell<usize>,
}

impl Find {
    /// Opens the bar with the keyboard in the field.
    pub fn open(&mut self) {
        self.open = true;
        self.focus = true;
        self.scroll_to_current.set(true);
    }

    pub fn close(&mut self) {
        self.open = false;
        self.current = 0;
        self.seen.set(0);
    }

    /// The next match, wrapping around the end.
    pub fn next(&mut self) {
        if self.total > 0 {
            self.current = (self.current + 1) % self.total;
            self.scroll_to_current.set(true);
        }
    }

    /// The previous match, wrapping around the start.
    pub fn previous(&mut self) {
        if self.total > 0 {
            self.current = (self.current + self.total - 1) % self.total;
            self.scroll_to_current.set(true);
        }
    }
}

/// Whether the find bar is looking for something right now.
pub fn is_searching(find: &Find) -> bool {
    find.open && !find.query.trim().is_empty()
}

/// The byte ranges of `needle` in `haystack`, case-insensitively, without overlapping.
///
/// Case folding is per character (each character against the needle's), which keeps the byte
/// offsets in the original string exact - what the reader sees highlighted is what the search found.
pub fn ranges(haystack: &str, needle: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let needle: Vec<char> = needle.trim().chars().collect();
    if needle.is_empty() || haystack.is_empty() {
        return out;
    }
    let mut at = 0;
    while at < haystack.len() {
        if let Some(length) = matches_at(&haystack[at..], &needle) {
            out.push((at, at + length));
            at += length;
        } else {
            at += haystack[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}

/// The byte length of `needle` at the very start of `rest`, when it is there.
fn matches_at(rest: &str, needle: &[char]) -> Option<usize> {
    let mut chars = rest.char_indices();
    let mut end = 0;
    for wanted in needle {
        let (at, found) = chars.next()?;
        if !found.to_lowercase().eq(wanted.to_lowercase()) {
            return None;
        }
        end = at + found.len_utf8();
    }
    Some(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_match_is_found_wherever_it_is_in_the_line() {
        assert_eq!(ranges("a Sepse e a sepse", "sepse"), [(2, 7), (12, 17)], "case-insensitive, both of them");
        assert_eq!(ranges("nada aqui", "sepse"), []);
        assert_eq!(ranges("abc", ""), [], "an empty needle finds nothing, not everything");
    }

    #[test]
    fn the_hint_holds_the_original_bytes() {
        let text = "Ação e ação";
        let found = ranges(text, "AÇÃO");
        assert_eq!(found.len(), 2);
        for (start, end) in found {
            assert_eq!(&text[start..end].to_lowercase(), "ação");
        }
    }

    #[test]
    fn walking_the_matches_asks_the_view_to_follow_once() {
        let mut find = Find { open: true, total: 3, ..Find::default() };
        find.open();
        assert!(find.scroll_to_current.replace(false), "opening brings the first match into sight");
        assert!(!find.scroll_to_current.replace(false), "the view takes the request once, not every frame");
        find.next();
        assert!(find.scroll_to_current.replace(false), "going to the next match follows it");
        find.previous();
        assert!(find.scroll_to_current.replace(false));
        find.total = 0;
        find.next();
        assert!(!find.scroll_to_current.replace(false), "nothing to walk, nothing to scroll");
    }

    #[test]
    fn walking_the_matches_wraps_around() {
        let mut find = Find { open: true, total: 3, ..Find::default() };
        find.next();
        find.next();
        assert_eq!(find.current, 2);
        find.next();
        assert_eq!(find.current, 0, "past the last one comes the first");
        find.previous();
        assert_eq!(find.current, 2, "and back");
        find.total = 0;
        find.next();
        assert_eq!(find.current, 2, "nothing to walk");
    }
}
