//! Rewriting `[[wikilinks]]` when a note is renamed, so the vault's links stay alive.
//!
//! A link's alias and anchor are the author's words and stay; only the last path segment of the
//! target is replaced (`[[Pasta/Nota|veja]]` → `[[Pasta/Novo|veja]]`). Code is not text: a
//! `[[Nota]]` inside a fence is an example, and rewriting it would corrupt documentation.

use crate::code;

/// Rewrites every link that names `old_name` to point at `new_name`. Returns the new content and
/// how many links changed.
pub fn rewrite(content: &str, old_name: &str, new_name: &str) -> (String, usize) {
    let mut count = 0;
    let out = code::map_outside_code(content, |chunk| rewrite_chunk(chunk, old_name, new_name, &mut count));
    (out, count)
}

/// How many links in `content` point at the note `name`.
pub fn count_refs(content: &str, name: &str) -> usize {
    rewrite(content, name, name).1
}

fn rewrite_chunk(chunk: &str, old_name: &str, new_name: &str, count: &mut usize) -> String {
    let mut out = String::with_capacity(chunk.len());
    let mut rest = chunk;
    while let Some(open) = rest.find("[[") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("]]") else {
            // An unclosed `[[` is text, not a link.
            out.push_str(&rest[open..]);
            return out;
        };
        let inner = &after[..close];
        out.push_str("[[");
        match rewritten_inner(inner, old_name, new_name) {
            Some(replaced) => {
                out.push_str(&replaced);
                *count += 1;
            }
            None => out.push_str(inner),
        }
        out.push_str("]]");
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    out
}

/// `inner` rewritten when it names `old_name`, `None` when it points somewhere else.
fn rewritten_inner(inner: &str, old_name: &str, new_name: &str) -> Option<String> {
    // `target#anchor|alias`, in that order: the alias runs to the closing brackets.
    let (target, alias) = match inner.find('|') {
        Some(at) => (&inner[..at], &inner[at..]),
        None => (inner, ""),
    };
    let (note, anchor) = match target.find('#') {
        Some(at) => (&target[..at], &target[at..]),
        None => (target, ""),
    };
    if !names_note(note, old_name) {
        return None;
    }
    // Keep the spaces the author wrote inside the brackets.
    let trimmed = note.trim();
    let lead = &note[..note.len() - note.trim_start().len()];
    let trail = &note[note.trim_end().len()..];
    Some(format!("{lead}{}{trail}{anchor}{alias}", replace_last_segment(trimmed, new_name)))
}

/// The link `[[Pasta/Nota]]` targets the note `Nota` — only the last segment is compared.
fn names_note(target: &str, name: &str) -> bool {
    let clean = strip_extension(target.trim());
    let last = clean.rsplit('/').next().unwrap_or(clean);
    last.to_lowercase() == name.trim().to_lowercase()
}

/// Replaces the last path segment of `target` with `new_name`, keeping any folder prefix.
fn replace_last_segment(target: &str, new_name: &str) -> String {
    let clean = strip_extension(target);
    match clean.rfind('/') {
        Some(at) => format!("{}{}", &clean[..=at], new_name),
        None => new_name.to_string(),
    }
}

fn strip_extension(target: &str) -> &str {
    let len = target.len();
    if len >= 3 && target.is_char_boundary(len - 3) && target[len - 3..].eq_ignore_ascii_case(".md") { &target[..len - 3] } else { target }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_a_plain_link() {
        assert_eq!(rewrite("veja [[Nota]]", "Nota", "Nova"), ("veja [[Nova]]".to_string(), 1));
    }

    #[test]
    fn keeps_the_alias_and_the_anchor() {
        assert_eq!(rewrite("[[Nota#Seção|a seção]]", "Nota", "Nova"), ("[[Nova#Seção|a seção]]".to_string(), 1));
    }

    #[test]
    fn keeps_the_folder_prefix() {
        assert_eq!(rewrite("[[Pasta/Nota]]", "Nota", "Nova"), ("[[Pasta/Nova]]".to_string(), 1));
    }

    #[test]
    fn keeps_the_spaces_inside_the_brackets() {
        assert_eq!(rewrite("[[ Nota ]]", "Nota", "Nova"), ("[[ Nova ]]".to_string(), 1));
    }

    #[test]
    fn matches_case_insensitively_and_ignores_the_extension() {
        assert_eq!(rewrite("[[nota.md]]", "Nota", "Nova"), ("[[Nova]]".to_string(), 1));
    }

    #[test]
    fn leaves_code_alone() {
        let text = "`[[Nota]]`\n```\n[[Nota]]\n```\n[[Nota]]";
        let (out, count) = rewrite(text, "Nota", "Nova");
        assert_eq!(count, 1, "{out}");
        assert!(out.starts_with("`[[Nota]]`"), "{out}");
    }

    #[test]
    fn leaves_other_notes_alone() {
        assert_eq!(rewrite("[[Outra]]", "Nota", "Nova"), ("[[Outra]]".to_string(), 0));
    }

    #[test]
    fn counts_references() {
        assert_eq!(count_refs("[[Nota]] e [[Pasta/Nota|x]]", "Nota"), 2);
    }

    #[test]
    fn an_unclosed_bracket_is_text() {
        assert_eq!(rewrite("[[Nota", "Nota", "Nova"), ("[[Nota".to_string(), 0));
    }
}
