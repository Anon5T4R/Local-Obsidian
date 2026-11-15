//! Splitting markdown into the stretches that are code and the ones that are not.
//!
//! Port of the upstream `MD_CODE_RE` split (`/```[\s\S]*?```|~~~[\s\S]*?~~~|`[^`\n]*`/`): a
//! fenced block runs to the next fence of the same marker, an inline span runs to the next
//! backtick on the same line, and everything else is plain markdown.
//!
//! Tags, wikilinks, embeds and link rewriting work only outside code: a `[[Note]]` inside a fence
//! is documentation, not a link, and `#ff0000` inside a code sample is not a tag.

/// `text` split into `(is_code, chunk)` runs, in order. Joining every chunk gives `text` back.
///
/// A fence or backtick with no closing partner is not code (the upstream regex needs both ends),
/// so it stays in the plain run and the scan keeps looking for a pair further on.
pub fn segments(text: &str) -> Vec<(bool, &str)> {
    let mut out: Vec<(bool, &str)> = Vec::new();
    let mut plain_from = 0;
    let mut i = 0;

    while i < text.len() {
        let rest = &text[i..];
        let closing = if rest.starts_with("```") || rest.starts_with("~~~") {
            let marker = &rest[..3];
            rest[3..].find(marker).map(|offset| i + 3 + offset + 3)
        } else if rest.starts_with('`') {
            // Inline code never spans a line break.
            let line_end = rest.find('\n').unwrap_or(rest.len());
            rest[1..line_end].find('`').map(|offset| i + 1 + offset + 1)
        } else {
            None
        };

        match closing {
            Some(end) => {
                if plain_from < i {
                    out.push((false, &text[plain_from..i]));
                }
                out.push((true, &text[i..end]));
                i = end;
                plain_from = end;
            }
            None => i += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    if plain_from < text.len() {
        out.push((false, &text[plain_from..]));
    }
    out
}

/// `f` applied to every stretch that is not code; code chunks are copied through untouched.
pub fn map_outside_code(text: &str, mut f: impl FnMut(&str) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    for (is_code, chunk) in segments(text) {
        if is_code {
            out.push_str(chunk);
        } else {
            out.push_str(&f(chunk));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> String {
        segments(text).into_iter().filter(|(code, _)| !code).map(|(_, chunk)| chunk).collect()
    }

    #[test]
    fn segments_cover_the_whole_text_in_order() {
        for text in ["", "plain", "a `code` b", "```\nx\n```", "a ```b``` c", "`unterminated", "```never closed"] {
            let joined: String = segments(text).into_iter().map(|(_, chunk)| chunk).collect();
            assert_eq!(joined, text, "{text:?} lost characters");
        }
    }

    #[test]
    fn a_fenced_block_is_code_and_so_is_an_inline_span() {
        let text = "a `x` b\n```\ncode\n```\nc";
        assert_eq!(plain(text), "a  b\n\nc");
        let code: Vec<&str> = segments(text).into_iter().filter(|(is_code, _)| *is_code).map(|(_, chunk)| chunk).collect();
        assert_eq!(code, vec!["`x`", "```\ncode\n```"]);
    }

    #[test]
    fn an_unclosed_fence_is_plain_text() {
        // The failed fence leaves "``", which the inline rule then reads as an empty inline span -
        // the same thing the upstream regex alternation does. Nothing is lost either way.
        assert_eq!(plain("```\nstill text"), "`\nstill text");
        assert_eq!(plain("`unterminated"), "`unterminated");
    }

    #[test]
    fn an_inline_span_stops_at_the_line_break() {
        let text = "a `b\nc` d";
        assert_eq!(plain(text), "a `b\nc` d");
    }

    #[test]
    fn map_outside_code_leaves_code_untouched() {
        let out = map_outside_code("[[A]] `[[B]]`", |chunk| chunk.replace("[[A]]", "<A>"));
        assert_eq!(out, "<A> `[[B]]`");
    }
}
