//! `#tags` in the note body, with the same shape the index and the renderer use, so what is
//! highlighted as a tag and what is listed under a tag never disagree.
//!
//! Port of the upstream `extractTags`: tags match across the whole Unicode range (a Brazilian
//! vault writes `#pré-natal`), `/` nests them (`#sistema/cardio`), and code is not text: a
//! `#ff0000` inside a snippet is a colour, not a tag.

use crate::code;

/// Characters a tag is made of: letters, digits, `_`, `-` and the `/` that nests it.
fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

/// Every tag in `content`, lowercase, without the `#`, deduplicated, in first-seen order.
pub fn extract(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (is_code, chunk) in code::segments(content) {
        if !is_code {
            scan(chunk, &mut out);
        }
    }
    out.dedup();
    out
}

/// Tags as the index stores them: `#sistema/cardio` also counts as `#sistema`, so a parent tag
/// finds its children.
pub fn expand_hierarchy(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let parts: Vec<&str> = tag.split('/').collect();
        for depth in 1..=parts.len() {
            let parent = parts[..depth].join("/");
            if !out.contains(&parent) {
                out.push(parent);
            }
        }
    }
    out
}

fn scan(chunk: &str, out: &mut Vec<String>) {
    let mut rest = chunk;
    while let Some(hash) = rest.find('#') {
        let after = &rest[hash + 1..];
        let end = after.find(|c: char| !is_tag_char(c)).unwrap_or(after.len());
        let raw = &after[..end];
        // Trailing separators come from prose ("veja #cardio/"), not from the tag.
        let tag = raw.trim_end_matches(['/', '-']).to_lowercase();
        if !tag.is_empty() && !is_numeric(&tag) && !is_hex_colour(&tag) && !out.contains(&tag) {
            out.push(tag);
        }
        if end >= after.len() {
            break;
        }
        rest = &after[end..];
    }
}

fn is_numeric(tag: &str) -> bool {
    tag.chars().all(|c| c.is_ascii_digit())
}

/// A bare colour literal (`#3b82f6`), which needs at least one digit so a word that happens to be
/// spelled with hex letters (`#cafe`, `#beefed`) stays a tag.
fn is_hex_colour(tag: &str) -> bool {
    matches!(tag.len(), 3 | 6 | 8) && tag.chars().all(|c| c.is_ascii_hexdigit()) && tag.chars().any(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(content: &str) -> Vec<String> {
        extract(content)
    }

    #[test]
    fn extracts_unique_lowercased_tags() {
        assert_eq!(tags("#Work some #work and #Idea"), vec!["work", "idea"]);
    }

    #[test]
    fn keeps_accented_tags_whole() {
        assert_eq!(tags("#pré-natal e #nutrição"), vec!["pré-natal", "nutrição"]);
    }

    #[test]
    fn keeps_nested_tags_whole() {
        assert_eq!(tags("#sistema/cardio"), vec!["sistema/cardio"]);
    }

    #[test]
    fn ignores_tags_inside_a_fenced_block() {
        assert_eq!(tags("```mermaid\nstyle A fill:#ff0000\n```\n#real"), vec!["real"]);
    }

    #[test]
    fn ignores_tags_inside_inline_code() {
        assert!(tags("use `#exemplo` aqui").is_empty());
    }

    #[test]
    fn ignores_a_bare_hex_colour() {
        assert!(tags("cor #3b82f6 e #ff0000").is_empty());
    }

    #[test]
    fn keeps_a_word_that_is_hex_like_but_has_no_digit() {
        assert_eq!(tags("#cafe #beefed"), vec!["cafe", "beefed"]);
    }

    #[test]
    fn ignores_a_purely_numeric_tag() {
        assert_eq!(tags("#2025 e #ano2025"), vec!["ano2025"]);
    }

    #[test]
    fn drops_a_trailing_separator_left_by_prose() {
        assert_eq!(tags("veja #cardio/ e #uti-"), vec!["cardio", "uti"]);
    }

    #[test]
    fn a_lone_hash_is_not_a_tag() {
        assert!(tags("número # e # ").is_empty());
    }

    #[test]
    fn hierarchy_adds_every_parent_once() {
        assert_eq!(expand_hierarchy(&["sistema/cardio/isquemia".to_string()]), vec!["sistema", "sistema/cardio", "sistema/cardio/isquemia"]);
        assert_eq!(expand_hierarchy(&["uti".to_string(), "uti".to_string()]), vec!["uti"]);
    }
}
