//! Wikilink syntax: `[[Note]]`, `[[Note|alias]]`, `[[Note#Section]]`, `[[Folder/Note]]`.
//!
//! Everything here is syntax only. Resolving a target to a file needs the vault (which knows the
//! notes), so it lives in `localnotebook-vault`; this module gives it the pieces: the target split
//! apart, the normalised form two notes are compared by, and the text a link shows.

/// A wikilink target split into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target<'a> {
    /// `Note` or `Folder/Note`: the part that names a file.
    pub note: &'a str,
    /// `Section` or `^block` after the `#`.
    pub hash: Option<&'a str>,
}

/// Splits `Nota#Seção` / `Pasta/Nota#^bloco` into its target and anchor parts.
pub fn parse_target(raw: &str) -> Target<'_> {
    match raw.find('#') {
        None => Target { note: raw.trim(), hash: None },
        Some(at) => {
            let hash = raw[at + 1..].trim();
            Target { note: raw[..at].trim(), hash: if hash.is_empty() { None } else { Some(hash) } }
        }
    }
}

/// The form two notes are compared by: `/` separators, no `.md`, lowercase.
pub fn norm(text: &str) -> String {
    let text = text.replace('\\', "/");
    let text = text.strip_suffix(".md").or_else(|| text.strip_suffix(".MD")).unwrap_or(&text);
    text.to_lowercase()
}

/// Every wikilink target in `content`, aliases stripped, in order.
///
/// Deliberately not code-aware, like the upstream index pass: the backlink index is built from the
/// raw note, and a link inside a snippet is still a link the author wrote.
pub fn extract(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = content;
    while let Some(open) = rest.find("[[") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("]]") else { break };
        let inner = &after[..close];
        let target = inner.split('|').next().unwrap_or(inner).trim();
        out.push(target.to_string());
        rest = &after[close + 2..];
    }
    out
}

/// `Nota#Seção` reads better as `Nota › Seção`; a block ref (`#^id`) is machine noise, so only the
/// note name is shown.
pub fn display_target(target: &str) -> String {
    let parsed = parse_target(target);
    match parsed.hash {
        None => parsed.note.to_string(),
        Some(hash) if hash.starts_with('^') => {
            if parsed.note.is_empty() {
                target.trim().to_string()
            } else {
                parsed.note.to_string()
            }
        }
        Some(hash) => {
            if parsed.note.is_empty() {
                hash.to_string()
            } else {
                format!("{} › {}", parsed.note, hash)
            }
        }
    }
}

/// The link destination a wikilink turns into in the preview: `#wikilink:<encoded target>`.
///
/// The scheme keeps the preview pipeline plain Markdown (pulldown-cmark turns it into a link like
/// any other) while the UI can tell a vault link from a web link by its prefix alone.
pub const URL_PREFIX: &str = "#wikilink:";

/// `target` (the raw text inside the brackets) as a preview link destination.
pub fn url(target: &str) -> String {
    format!("{URL_PREFIX}{}", encode(target))
}

/// The raw target inside a preview link destination, if `destination` is a wikilink.
pub fn from_url(destination: &str) -> Option<String> {
    destination.strip_prefix(URL_PREFIX).map(decode)
}

fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(byte) = hex {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_splits_at_the_hash() {
        assert_eq!(parse_target("Nota#Seção"), Target { note: "Nota", hash: Some("Seção") });
        assert_eq!(parse_target("Pasta/Nota#^bloco"), Target { note: "Pasta/Nota", hash: Some("^bloco") });
        assert_eq!(parse_target("Nota"), Target { note: "Nota", hash: None });
        assert_eq!(parse_target("Nota#"), Target { note: "Nota", hash: None });
    }

    #[test]
    fn norm_folds_case_separators_and_the_extension() {
        assert_eq!(norm("Pasta\\Nota.MD"), "pasta/nota");
        assert_eq!(norm("Nota"), "nota");
    }

    #[test]
    fn extract_strips_aliases_and_keeps_order() {
        assert_eq!(extract("see [[Alpha]] and [[Beta|B]]"), vec!["Alpha", "Beta"]);
    }

    #[test]
    fn extract_keeps_the_anchor() {
        assert_eq!(extract("[[Nota#Seção]]"), vec!["Nota#Seção"]);
    }

    #[test]
    fn display_shows_the_section_after_an_arrow() {
        assert_eq!(display_target("Nota#Seção"), "Nota › Seção");
        assert_eq!(display_target("Nota"), "Nota");
        assert_eq!(display_target("Nota#^bloco"), "Nota");
        assert_eq!(display_target("#Seção"), "Seção");
    }

    #[test]
    fn a_url_round_trips_every_target() {
        for target in ["Nota", "Pasta/Nota#Seção", "Nota com espaço", "nota(1)", "Nota#^bloco"] {
            let url = url(target);
            assert_eq!(from_url(&url).as_deref(), Some(target), "{url}");
        }
    }
}
