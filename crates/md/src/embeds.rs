//! Transclusion: `![[Note]]` / `![[Note#Section]]`.
//!
//! Without it, content used in ten notes has to be copied ten times and the copies drift apart.
//! The embedded note is pasted in as a quoted block whose first line names where it came from, so
//! the reader can always tell the embed from the note's own text; nesting stops at
//! [`MAX_DEPTH`] and a cycle (A embeds B embeds A) is cut instead of recursing forever.

use crate::code;
use crate::frontmatter;
use crate::links::{self, parse_target};

/// How deep one embed may nest another.
pub const MAX_DEPTH: usize = 3;

/// Markdown of a note, or `None` when there is no note behind the target.
pub type Resolver<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// Replaces every `![[…]]` with the content it points at.
pub fn expand(markdown: &str, resolve: &Resolver<'_>) -> String {
    expand_at(markdown, resolve, 0, &mut Vec::new())
}

fn expand_at(markdown: &str, resolve: &Resolver<'_>, depth: usize, seen: &mut Vec<String>) -> String {
    if depth >= MAX_DEPTH {
        return markdown.to_string();
    }
    // An `![[…]]` inside code is documentation: injecting a quote there would wreck the fence.
    code::map_outside_code(markdown, |chunk| expand_chunk(chunk, resolve, depth, seen))
}

fn expand_chunk(chunk: &str, resolve: &Resolver<'_>, depth: usize, seen: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(chunk.len());
    let mut rest = chunk;
    while let Some(open) = rest.find("![[") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 3..];
        let Some(close) = after.find("]]") else {
            out.push_str(&rest[open..]);
            return out;
        };
        let inner = &after[..close];
        out.push_str(&embedded(inner, resolve, depth, seen));
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    out
}

/// The markdown an `![[inner]]` becomes.
fn embedded(inner: &str, resolve: &Resolver<'_>, depth: usize, seen: &mut Vec<String>) -> String {
    // `![[Note|30]]`: the part after `|` is a sizing hint, not part of the target.
    let raw = inner.split('|').next().unwrap_or(inner);
    // `![[foto.png]]` is an attachment, not a transclusion: the preview draws it as an image.
    if is_attachment(raw) {
        return format!("![[{inner}]]");
    }
    let parsed = parse_target(raw);
    let key = raw.trim().to_lowercase();

    if seen.contains(&key) {
        return broken(raw);
    }
    let Some(resolved) = resolve(parsed.note) else { return broken(raw) };
    // The embedded note is no longer at the top of the document, so its own frontmatter would
    // render as body text: drop it here.
    let source = frontmatter::body(&resolved);
    let body = match parsed.hash {
        Some(section) if !section.starts_with('^') => extract_section(source, section),
        _ => source.to_string(),
    };
    if body.trim().is_empty() {
        return broken(raw);
    }

    seen.push(key);
    let inner_markdown = expand_at(&body, resolve, depth + 1, seen);
    seen.pop();

    let mut out = format!("\n\n> [!embed] {}\n>\n", links::display_target(raw.trim()));
    for line in inner_markdown.trim_end().lines() {
        if line.is_empty() {
            out.push_str(">\n");
        } else {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str("\n\n");
    out
}

/// A target with no note behind it: keep the literal `![[…]]` visible, escaped so the wikilink
/// pass does not turn it into a half-broken link.
fn broken(raw: &str) -> String {
    format!("!\\[\\[{}\\]\\]", raw.trim())
}

/// Whether the target names a file that is not a note (an image, a PDF): those are attachments,
/// and the embed pass must leave them for the image pass.
fn is_attachment(target: &str) -> bool {
    let target = target.trim();
    let Some(extension) = target.rsplit_once('.').map(|(_, extension)| extension.to_lowercase()) else {
        return false;
    };
    !extension.is_empty() && extension != "md" && extension.chars().all(|c| c.is_ascii_alphanumeric())
}

/// The slice of `markdown` under the heading whose text matches `heading`, up to the next heading
/// of the same or higher level. Empty when there is no such heading.
pub fn extract_section(markdown: &str, heading: &str) -> String {
    let wanted = heading.trim().to_lowercase();
    let mut section: Vec<&str> = Vec::new();
    let mut level = 0;
    let mut inside = false;

    for line in markdown.lines() {
        let (hashes, text) = match heading_of(line) {
            Some(parts) => parts,
            None => {
                if inside {
                    section.push(line);
                }
                continue;
            }
        };
        if !inside {
            if text.trim().to_lowercase() == wanted {
                inside = true;
                level = hashes;
                section.push(line);
            }
            continue;
        }
        if hashes <= level {
            break;
        }
        section.push(line);
    }
    section.join("\n").trim().to_string()
}

/// `(level, text)` when the line is an ATX heading.
fn heading_of(line: &str) -> Option<(usize, &str)> {
    let hashes = line.len() - line.trim_start_matches('#').len();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    let text = rest.strip_prefix(' ')?;
    Some((hashes, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vault of two notes: `Nota` embeds nothing, `Alvo` has two sections.
    fn resolver(target: &str) -> Option<String> {
        match target {
            "Nota" => Some("---\ntags: [x]\n---\nconteúdo".to_string()),
            "Alvo" => Some("# Um\n\ntexto um\n\n## Dois\n\ntexto dois".to_string()),
            "A" => Some("a antes ![[B]]".to_string()),
            "B" => Some("b antes ![[A]]".to_string()),
            _ => None,
        }
    }

    #[test]
    fn an_embed_is_replaced_by_the_note_without_its_header() {
        let out = expand("início ![[Nota]] fim", &resolver);
        assert!(out.contains("conteúdo"), "{out}");
        assert!(!out.contains("tags: [x]"), "{out}");
        assert!(out.contains("[!embed] Nota"), "{out}");
    }

    #[test]
    fn a_section_embed_takes_only_that_section() {
        let out = expand("![[Alvo#Dois]]", &resolver);
        assert!(out.contains("texto dois"), "{out}");
        assert!(!out.contains("texto um"), "{out}");
    }

    #[test]
    fn a_missing_target_stays_visible() {
        let out = expand("![[Sumiu]]", &resolver);
        assert_eq!(out, "!\\[\\[Sumiu\\]\\]");
    }

    #[test]
    fn a_cycle_is_cut() {
        let out = expand("![[A]]", &resolver);
        assert!(out.contains("a antes"), "{out}");
        // `A -> B -> A`: the inner A is refused, and the expander terminates.
        assert!(out.contains("!\\[\\[A\\]\\]"), "{out}");
    }

    #[test]
    fn code_is_not_an_embed() {
        let text = "`![[Nota]]`\n```\n![[Nota]]\n```";
        assert_eq!(expand(text, &resolver), text);
    }

    #[test]
    fn a_heading_of_the_same_level_ends_the_section() {
        assert_eq!(extract_section("# Um\n\ncorpo\n\n# Outro\n\nnada", "Um"), "# Um\n\ncorpo");
        assert_eq!(extract_section("## Dois\n\na\n### Três\n\nb\n## Fim", "Dois"), "## Dois\n\na\n### Três\n\nb");
    }

    #[test]
    fn a_section_that_does_not_exist_is_empty() {
        assert!(extract_section("# Um\n\ntexto", "Nada").is_empty());
    }

    #[test]
    fn a_block_reference_is_not_a_section() {
        let out = expand("![[Nota#^bloco]]", &resolver);
        assert!(out.contains("conteúdo"), "{out}");
    }

    #[test]
    fn an_attachment_is_left_for_the_image_pass() {
        assert_eq!(expand("![[gato.png]]", &resolver), "![[gato.png]]");
        assert_eq!(expand("![[foto.jpg|300]]", &resolver), "![[foto.jpg|300]]");
    }
}
