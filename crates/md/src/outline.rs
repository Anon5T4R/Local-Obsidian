//! The note's outline: its headings, in order, with the anchor each one answers to.
//!
//! Port of the upstream `TocPanel`'s `extractHeadings`: ATX headings inside fenced code are not
//! headings (a `# comment` in a shell snippet is documentation), levels 1-3 make the outline, and a
//! repeated title gets a numbered anchor so two "## Conduta" sections stay addressable.
//!
//! [`slug`] is also what a `[[Note#Section]]` link is matched by, so a link and its heading always
//! agree on which section they mean.

/// One heading of the note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// 1 for `#`, 2 for `##`, 3 for `###`.
    pub level: u8,
    /// The heading's text, as written.
    pub text: String,
    /// The anchor: [`slug`] of the text, numbered when it repeats.
    pub anchor: String,
}

/// The outline of `markdown`, in document order.
pub fn outline(markdown: &str) -> Vec<Heading> {
    let mut headings: Vec<Heading> = Vec::new();
    let mut seen: Vec<(String, usize)> = Vec::new();
    let mut in_code = false;

    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        let Some((level, text)) = atx_heading(line) else { continue };
        if level > 3 {
            continue;
        }
        let base = slug(&text);
        let anchor = match seen.iter_mut().find(|(name, _)| *name == base) {
            Some((name, count)) => {
                *count += 1;
                format!("{name}-{count}")
            }
            None => {
                seen.push((base.clone(), 0));
                base
            }
        };
        headings.push(Heading { level, text, anchor });
    }
    headings
}

/// `(level, text)` when the line is an ATX heading.
fn atx_heading(line: &str) -> Option<(u8, String)> {
    let trimmed = line.trim_start();
    let hashes = trimmed.len() - trimmed.trim_start_matches('#').len();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = trimmed[hashes..].strip_prefix(' ')?;
    let text = rest.trim_end().trim_end_matches('#').trim_end().to_string();
    (!text.is_empty()).then_some((hashes as u8, text))
}

/// A heading's anchor: lowercase, no punctuation, spaces to `-`.
///
/// Accents are kept (unlike the upstream's ASCII `\w`): the anchor is never written to a file, it is
/// only compared with the anchor of a `[[Note#Section]]`, and both sides run through this function.
pub fn slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_dash = false;
    for c in text.to_lowercase().chars() {
        if c.is_alphanumeric() || c == '_' {
            out.push(c);
            last_dash = false;
        } else if (c.is_whitespace() || c == '-') && !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
        // Everything else (punctuation) disappears.
    }
    out.trim_end_matches('-').to_string()
}

/// Whether `section` (the text after the `#` in a wikilink) names `heading`.
pub fn section_matches(section: &str, heading: &str) -> bool {
    let section = section.trim();
    if section.eq_ignore_ascii_case(heading.trim()) {
        return true;
    }
    slug(section) == slug(heading)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels(headings: &[Heading]) -> Vec<u8> {
        headings.iter().map(|heading| heading.level).collect()
    }

    fn texts(headings: &[Heading]) -> Vec<&str> {
        headings.iter().map(|heading| heading.text.as_str()).collect()
    }

    #[test]
    fn an_outline_keeps_the_headings_in_order() {
        let headings = outline("# Um\n\ntexto\n\n## Dois\n\n### Três\n");
        assert_eq!(levels(&headings), [1, 2, 3]);
        assert_eq!(texts(&headings), ["Um", "Dois", "Três"]);
    }

    #[test]
    fn only_the_first_three_levels_make_the_outline() {
        let headings = outline("# Um\n#### Quatro\n##### Cinco\n");
        assert_eq!(texts(&headings), ["Um"]);
    }

    #[test]
    fn a_heading_inside_a_fence_is_not_a_heading() {
        let headings = outline("```bash\n# não é título\n```\n# Real\n");
        assert_eq!(texts(&headings), ["Real"]);
    }

    #[test]
    fn a_repeated_title_gets_a_numbered_anchor() {
        let headings = outline("# Conduta\n\n## Conduta\n\n## Conduta\n");
        assert_eq!(headings.iter().map(|h| h.anchor.as_str()).collect::<Vec<_>>(), ["conduta", "conduta-1", "conduta-2"]);
    }

    #[test]
    fn a_hash_without_a_space_is_not_a_heading() {
        assert!(outline("#nao\n##tambem").is_empty());
    }

    #[test]
    fn trailing_hashes_are_not_part_of_the_title() {
        assert_eq!(texts(&outline("## Título ##")), ["Título"]);
    }

    #[test]
    fn the_slug_keeps_letters_and_accents_and_drops_punctuation() {
        assert_eq!(slug("Sepse: conduta (2026)"), "sepse-conduta-2026");
        assert_eq!(slug("Avaliação  inicial"), "avaliação-inicial");
        assert_eq!(slug("sistema/cardio"), "sistemacardio");
    }

    #[test]
    fn a_section_link_matches_the_heading_it_names() {
        assert!(section_matches("Conduta", "Conduta"));
        assert!(section_matches("conduta", "Conduta"));
        assert!(section_matches("Avaliação inicial", "Avaliação  inicial"));
        assert!(!section_matches("Conduta", "Condutas"));
    }
}
