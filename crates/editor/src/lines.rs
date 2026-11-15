//! Line-level edits: list markers, quote markers, headings, indentation and task checkboxes.
//!
//! Everything here works on whole lines the selection touches, adds or removes a prefix on every
//! one of them, and keeps the selection over the same words afterwards - so pressing the shortcut
//! twice leaves the note exactly as it was.

use crate::text::{Edit, Selection, line_end, line_start, touched_lines};

/// The bullet the editor writes for a plain list item.
pub const BULLET: &str = "- ";
/// One level of indentation. Two spaces, which is what Obsidian nests with.
pub const INDENT: &str = "  ";
/// The quote marker, with its trailing space.
pub const QUOTE: &str = "> ";

/// Adds `prefix` to every touched line, or takes it off when they all have it already.
pub fn toggle_prefix(text: &str, selection: Selection, prefix: &str) -> Edit {
    let selection = selection.clamped(text);
    let lines = touched_lines(text, selection);
    let all_have = !lines.is_empty() && lines.iter().all(|(start, end)| text[*start..*end].starts_with(prefix));
    let changes: Vec<(usize, bool)> = lines.iter().map(|(start, _)| (*start, !all_have)).collect();
    apply_prefix(text, selection, prefix, &changes)
}

/// Turns the touched lines into a numbered list (`1. `, `2. `, … on each line), or takes the
/// numbering off when every line already has one.
pub fn toggle_numbered(text: &str, selection: Selection) -> Edit {
    let selection = selection.clamped(text);
    let lines = touched_lines(text, selection);
    let numbered = |line: &str| {
        let trimmed = line.trim_start();
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        digits > 0 && trimmed[digits..].starts_with(". ")
    };
    let all_have = !lines.is_empty() && lines.iter().all(|(start, end)| numbered(&text[*start..*end]));

    // Numbered prefixes differ per line, so this one is rebuilt rather than delegated.
    let mut out = String::with_capacity(text.len() + lines.len() * 3);
    let mut cursor = selection.start;
    let mut copied = 0;
    for (index, (start, end)) in lines.iter().enumerate() {
        out.push_str(&text[copied..*start]);
        let line = &text[*start..*end];
        if all_have {
            let trimmed = line.trim_start();
            let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
            let lead = line.len() - trimmed.len();
            out.push_str(&line[..lead]);
            out.push_str(&trimmed[digits + 2..]);
            cursor = shift(cursor, *start + lead, digits + 2, false);
        } else {
            let marker = format!("{}. ", index + 1);
            out.push_str(&marker);
            out.push_str(line);
            cursor = shift(cursor, *start, marker.len(), true);
        }
        copied = *end;
    }
    out.push_str(&text[copied..]);
    Edit { text: out, selection: Selection::caret(cursor) }
}

/// Adds the next heading level to the touched lines: `# `, then `## `, then `### `, then back to
/// plain text. Not a prefix toggle: a heading is a level, not a decoration.
pub fn cycle_heading(text: &str, selection: Selection) -> Edit {
    let selection = selection.clamped(text);
    let lines = touched_lines(text, selection);
    let level_of = |line: &str| line.trim_start().chars().take_while(|c| *c == '#').count();
    let levels: Vec<usize> = lines.iter().map(|(start, end)| level_of(&text[*start..*end])).collect();
    // Every line at the same level goes one step down the cycle: plain -> # -> ## -> ### -> plain.
    let level = levels.first().copied().unwrap_or(0);
    let next = if levels.iter().all(|l| *l == level) && level < 3 { level + 1 } else { 0 };

    let mut out = String::with_capacity(text.len() + lines.len() * 2);
    let mut cursor = selection.start;
    let mut copied = 0;
    for (start, end) in &lines {
        out.push_str(&text[copied..*start]);
        let line = &text[*start..*end];
        let old = level_of(line);
        let trimmed = line.trim_start_matches('#');
        let body = trimmed.strip_prefix(' ').unwrap_or(trimmed);
        if old > 0 {
            cursor = shift(cursor, *start, old + trimmed.len() - body.len() - old, false);
        }
        if next > 0 {
            out.push_str(&"#".repeat(next));
            out.push(' ');
            cursor = shift(cursor, *start, next + 1, true);
        }
        out.push_str(body);
        cursor += 0;
        copied = *end;
    }
    out.push_str(&text[copied..]);
    Edit { text: out, selection: Selection::caret(cursor) }
}

/// Indents (`outdent` off) or outdents every touched line by one level.
pub fn indent(text: &str, selection: Selection, outdent: bool) -> Edit {
    let selection = selection.clamped(text);
    let lines = touched_lines(text, selection);
    if outdent {
        let changes: Vec<(usize, bool)> = lines.iter().map(|(start, _)| (*start, false)).collect();
        return apply_prefix(text, selection, INDENT, &changes);
    }
    let changes: Vec<(usize, bool)> = lines.iter().map(|(start, _)| (*start, true)).collect();
    apply_prefix(text, selection, INDENT, &changes)
}

/// What Enter should insert when the caret sits at the end of a list item: the next item's marker,
/// or - on an empty item - nothing, which ends the list.
pub fn list_continuation(text: &str, caret: usize) -> Option<Edit> {
    let caret = crate::text::floor_boundary(text, caret);
    let start = line_start(text, caret);
    let end = line_end(text, caret);
    // Only at the end of the line: pressing Enter mid-item splits the text, as usual.
    if caret != end {
        return None;
    }
    let line = &text[start..end];
    let (lead, marker, body) = item_parts(line)?;

    // An empty item ends the list: the marker goes away and the line stays empty.
    if body.is_empty() {
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..start]);
        out.push_str(&text[end..]);
        return Some(Edit { text: out, selection: Selection::caret(start) });
    }

    let mut next = String::with_capacity(lead.len() + marker.len() + 2);
    next.push_str(&lead);
    next.push_str(&marker);
    let mut out = String::with_capacity(text.len() + next.len() + 1);
    out.push_str(&text[..caret]);
    out.push('\n');
    out.push_str(&next);
    out.push_str(&text[caret..]);
    Some(Edit { text: out, selection: Selection::caret(caret + 1 + next.len()) })
}

/// `(indent, marker, body)` when the line is a list, quote or task item; the marker is the text to
/// repeat on the next line.
fn item_parts(line: &str) -> Option<(String, String, &str)> {
    let lead = &line[..line.len() - line.trim_start().len()];
    let rest = &line[lead.len()..];

    for marker in ["- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "- ", "* ", "+ "] {
        if let Some(body) = rest.strip_prefix(marker) {
            // A task continues as an unchecked task even when the finished one was checked.
            let marker = match marker {
                "- [x] " | "- [X] " => "- [ ] ",
                other => other,
            };
            return Some((lead.to_string(), marker.to_string(), body));
        }
    }
    if let Some(body) = rest.strip_prefix(QUOTE) {
        return Some((lead.to_string(), QUOTE.to_string(), body));
    }
    // `12. item`: the next item keeps counting.
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let after = &rest[digits..];
        if let Some(body) = after.strip_prefix(". ") {
            let number = rest[..digits].parse::<u64>().unwrap_or(0);
            return Some((lead.to_string(), format!("{}. ", number + 1), body));
        }
    }
    None
}

/// What Enter adds after a list item, and whether the item it follows was empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextItem {
    /// The marker the next line starts with (`- `, `  - `, `8. `, `> `).
    pub marker: String,
    /// The indentation the marker carries, in bytes.
    pub lead_len: usize,
    /// The item it follows had no text: pressing Enter there ends the list.
    pub empty: bool,
}

/// The marker a new item gets after `line`, when `line` is a list, quote or task item.
pub fn next_item(line: &str) -> Option<NextItem> {
    let (lead, marker, body) = item_parts(line)?;
    Some(NextItem { marker: format!("{lead}{marker}"), lead_len: lead.len(), empty: body.trim().is_empty() })
}

/// Flips the task checkbox inside `range` (one line, usually), when that line is a task.
pub fn toggle_line_task(text: &str, range: std::ops::Range<usize>) -> Option<Edit> {
    let start = crate::text::line_start(text, range.start);
    let end = crate::text::line_end(text, range.end.min(text.len()));
    let line = &text[start..end];
    let (offset, marker) = flip_box(line)?;
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..start + offset]);
    out.push_str(marker);
    out.push_str(&text[start + offset + BOX_LEN..]);
    Some(Edit { text: out, selection: Selection::caret(start + offset + marker.len()) })
}

/// The length of a task marker (`[ ] `), all the spellings the flip accepts.
const BOX_LEN: usize = 4;

/// `(offset of the box in the line, the marker that replaces it)` when the line is a task.
fn flip_box(line: &str) -> Option<(usize, &'static str)> {
    let lead = line.len() - line.trim_start().len();
    let rest = &line[lead..];
    let bullet = ["- ", "* ", "+ "].iter().find(|bullet| rest.starts_with(*bullet))?;
    let offset = lead + bullet.len();
    let body = &line[offset..];
    if body.starts_with("[ ] ") {
        Some((offset, "[x] "))
    } else if body.starts_with("[x] ") || body.starts_with("[X] ") {
        Some((offset, "[ ] "))
    } else {
        None
    }
}

/// Flips the `task_index`-th task checkbox of the note, by source order (the reading view knows
/// which box was clicked, and this is the source edit behind it).
pub fn toggle_task(text: &str, task_index: usize) -> Option<Edit> {
    let mut seen = 0;
    for (start, _) in touched_lines(text, Selection::new(0, text.len())) {
        let end = line_end(text, start);
        let Some((offset, marker)) = flip_box(&text[start..end]) else { continue };
        if seen == task_index {
            let box_start = start + offset;
            let mut out = String::with_capacity(text.len());
            out.push_str(&text[..box_start]);
            out.push_str(marker);
            out.push_str(&text[box_start + BOX_LEN..]);
            return Some(Edit { text: out, selection: Selection::caret(box_start + marker.len()) });
        }
        seen += 1;
    }
    None
}

/// The common prefix edit: `changes` says, per line start, whether to add or remove `prefix`.
fn apply_prefix(text: &str, selection: Selection, prefix: &str, changes: &[(usize, bool)]) -> Edit {
    let mut out = String::with_capacity(text.len() + changes.len() * prefix.len());
    let mut cursor = selection.start;
    let mut copied = 0;
    for (start, add) in changes {
        let end = line_end(text, *start);
        out.push_str(&text[copied..*start]);
        if *add {
            out.push_str(prefix);
            if cursor >= *start {
                cursor += prefix.len();
            }
            out.push_str(&text[*start..end]);
        } else if text[*start..end].starts_with(prefix) {
            out.push_str(&text[*start + prefix.len()..end]);
            cursor = shift(cursor, *start, prefix.len(), false);
        } else {
            // Nothing to take off on this line (a line shorter than the selection, an empty line).
            out.push_str(&text[*start..end]);
        }
        copied = end;
    }
    out.push_str(&text[copied..]);
    Edit { text: out, selection: Selection::caret(cursor) }
}

/// Moves `position` when `len` bytes are inserted (`adding`) or removed at `at`.
fn shift(position: usize, at: usize, len: usize, adding: bool) -> usize {
    if adding {
        if position >= at { position + len } else { position }
    } else if position >= at + len {
        position - len
    } else if position > at {
        at
    } else {
        position
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caret(at: usize) -> Selection {
        Selection::caret(at)
    }

    #[test]
    fn a_bullet_goes_on_every_touched_line_and_off_again() {
        let text = "um\ndois";
        let edit = toggle_prefix(text, Selection::new(0, 6), BULLET);
        assert_eq!(edit.text, "- um\n- dois");
        let edit = toggle_prefix(&edit.text, Selection::new(0, 10), BULLET);
        assert_eq!(edit.text, "um\ndois");
    }

    #[test]
    fn a_numbered_list_counts_up_and_strips_back() {
        let edit = toggle_numbered("um\ndois\ntrês", Selection::new(0, 12));
        assert_eq!(edit.text, "1. um\n2. dois\n3. três");
        let back = toggle_numbered(&edit.text, Selection::new(0, edit.text.len()));
        assert_eq!(back.text, "um\ndois\ntrês");
    }

    #[test]
    fn the_heading_cycles_through_the_levels() {
        let once = cycle_heading("Título", caret(0));
        assert_eq!(once.text, "# Título");
        let twice = cycle_heading(&once.text, caret(0));
        assert_eq!(twice.text, "## Título");
        let thrice = cycle_heading(&twice.text, caret(0));
        assert_eq!(thrice.text, "### Título");
        let plain = cycle_heading(&thrice.text, caret(0));
        assert_eq!(plain.text, "Título");
    }

    #[test]
    fn indenting_adds_two_spaces_and_outdenting_takes_them_off() {
        let edit = indent("um\ndois", Selection::new(0, 6), false);
        assert_eq!(edit.text, "  um\n  dois");
        let back = indent(&edit.text, Selection::new(0, 10), true);
        assert_eq!(back.text, "um\ndois");
    }

    #[test]
    fn enter_on_a_list_item_starts_the_next_one() {
        let text = "- leite";
        let edit = list_continuation(text, text.len()).expect("a list item");
        assert_eq!(edit.text, "- leite\n- ");
        assert_eq!(edit.selection.start, edit.text.len());
    }

    #[test]
    fn enter_on_an_empty_item_ends_the_list() {
        let text = "- leite\n- ";
        let edit = list_continuation(text, text.len()).expect("an empty item");
        assert_eq!(edit.text, "- leite\n");
    }

    #[test]
    fn a_task_continues_unchecked_and_a_number_keeps_counting() {
        let edit = list_continuation("- [x] feito", 11).expect("a task");
        assert_eq!(edit.text, "- [x] feito\n- [ ] ");
        let edit = list_continuation("7. sete", 7).expect("a numbered item");
        assert_eq!(edit.text, "7. sete\n8. ");
    }

    #[test]
    fn a_quote_continues_and_plain_text_does_not() {
        let quote = "> citação";
        assert_eq!(list_continuation(quote, quote.len()).map(|e| e.text), Some("> citação\n> ".to_string()));
        assert!(list_continuation("texto", 5).is_none());
    }

    #[test]
    fn enter_in_the_middle_of_an_item_does_nothing_special() {
        assert!(list_continuation("- leite", 3).is_none());
    }

    #[test]
    fn a_checkbox_flips_by_source_order() {
        let text = "- [ ] a\n- [x] b\n- [ ] c";
        let edit = toggle_task(text, 1).expect("second box");
        assert_eq!(edit.text, "- [ ] a\n- [ ] b\n- [ ] c");
        let edit = toggle_task(text, 2).expect("third box");
        assert_eq!(edit.text, "- [ ] a\n- [x] b\n- [x] c");
        assert!(toggle_task(text, 9).is_none());
    }

    #[test]
    fn an_item_with_indentation_keeps_it_on_the_next_line() {
        let edit = list_continuation("  - leite", 9).expect("an indented item");
        assert_eq!(edit.text, "  - leite\n  - ");
    }
}
