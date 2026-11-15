//! `> [!warning] Careful` - the callout syntax the Markdown note ecosystem shares (Obsidian,
//! GitHub-flavoured Markdown), plus this app's own `[!embed]` head.
//!
//! A quote stays a quote: only the one whose first line is `[!kind]` becomes a callout. The type is
//! kept as written (lowercase), the title falls back to the type's name, and a `+`/`-` right after
//! the type is the fold marker of the syntax - the reading view shows the body either way.
//!
//! The header and the body may arrive in the same paragraph (the `prepare` pass separates them with
//! a hard break), and the body may carry links or emphasis: the header is cut out of the first text
//! run and the rest stays as the callout's first paragraph.

use super::{Block, Span};

/// Whether `text` names a callout type: letters, digits, `-` and `_`.
fn is_kind(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// Promotes the quotes that carry a callout header, recursively.
pub(super) fn promote(blocks: Vec<Block>) -> Vec<Block> {
    blocks
        .into_iter()
        .map(|block| match block {
            Block::Quote(mut inner) => {
                let promoted = promote(inner);
                inner = promoted;
                match take_header(&mut inner) {
                    Some((kind, title)) => Block::Callout { kind, title, body: inner },
                    None => Block::Quote(inner),
                }
            }
            Block::Callout { kind, title, body } => Block::Callout { kind, title, body: promote(body) },
            other => other,
        })
        .collect()
}

/// `(kind, title)` when the first block is a callout header; that line is then removed from `inner`.
///
/// The header and the body of the callout may share one paragraph (`prepare` puts a hard break
/// between them), and the body may carry links and emphasis - so the header is taken out of the
/// first text run, and whatever follows it stays as the callout's first paragraph.
fn take_header(inner: &mut Vec<Block>) -> Option<(String, String)> {
    let Block::Paragraph(spans) = inner.first()? else { return None };
    let (first, rest_spans) = spans.split_first()?;
    let Span::Text(first_text) = first else { return None };

    // The header is the first line: the text run that opens it, plus whatever inline runs follow
    // until the line break that `prepare` inserted.
    let (head, after_break) = match first_text.split_once('\n') {
        Some((head, rest)) => (head, Some(rest)),
        None => (first_text.as_str(), None),
    };
    let head = head.trim_start().strip_prefix("[!")?;
    let close = head.find(']')?;
    let kind = head[..close].trim().to_lowercase();
    if !is_kind(&kind) {
        return None;
    }

    let mut title_parts: Vec<String> = Vec::new();
    let mut body_spans: Vec<Span> = Vec::new();
    let head_tail = head[close + 1..].trim().trim_end_matches('\\').trim();
    // A `+`/`-` right after the type is the fold marker of the syntax, not a title.
    let head_tail = head_tail.strip_prefix(['+', '-']).unwrap_or(head_tail).trim();
    if !head_tail.is_empty() {
        title_parts.push(head_tail.to_string());
    }
    let mut in_body = after_break.is_some();
    if let Some(after_break) = after_break.map(str::trim_start).filter(|rest| !rest.is_empty()) {
        body_spans.push(Span::Text(after_break.to_string()));
    }
    for span in rest_spans {
        if in_body {
            body_spans.push(span.clone());
            continue;
        }
        match span {
            Span::Text(text) => match text.split_once('\n') {
                Some((head, rest)) => {
                    title_parts.push(head.trim().to_string());
                    if !rest.trim().is_empty() {
                        body_spans.push(Span::Text(rest.trim_start().to_string()));
                    }
                    in_body = true;
                }
                None => title_parts.push(text.trim().to_string()),
            },
            other => title_parts.push(other.plain()),
        }
    }

    let title: String = title_parts.join(" ").trim().to_string();
    let title = if title.is_empty() { title_case(&kind) } else { title };

    if body_spans.is_empty() {
        inner.remove(0);
    } else {
        inner[0] = Block::Paragraph(body_spans);
    }
    Some((kind, title))
}

fn title_case(kind: &str) -> String {
    let mut chars = kind.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn callout(quote: &str) -> Block {
        let blocks = super::super::blocks(&super::super::prepare(quote));
        blocks.into_iter().next().expect("one block")
    }

    #[test]
    fn a_typed_header_becomes_a_callout() {
        let Block::Callout { kind, title, body } = callout("> [!warning] Cuidado\n>\n> texto") else { panic!("not a callout") };
        assert_eq!(kind, "warning");
        assert_eq!(title, "Cuidado");
        assert_eq!(body.len(), 1);
        assert_eq!(body[0].plain(), "texto");
    }

    #[test]
    fn the_title_falls_back_to_the_type_name() {
        let Block::Callout { kind, title, .. } = callout("> [!note]\n> corpo") else { panic!("not a callout") };
        assert_eq!((kind.as_str(), title.as_str()), ("note", "Note"));
    }

    #[test]
    fn the_fold_marker_is_not_part_of_the_title() {
        for head in ["> [!tip]-", "> [!tip]+ Aberto"] {
            let Block::Callout { title, .. } = callout(head) else { panic!("not a callout: {head}") };
            assert!(title == "Tip" || title == "Aberto", "{head} -> {title}");
        }
    }

    #[test]
    fn an_embed_head_is_a_callout_too() {
        let Block::Callout { kind, title, .. } = callout("> [!embed] Alvo › Um\n>\n> corpo") else { panic!("not a callout") };
        assert_eq!((kind.as_str(), title.as_str()), ("embed", "Alvo › Um"));
    }

    #[test]
    fn a_plain_quote_stays_a_quote() {
        assert!(matches!(callout("> só uma citação"), Block::Quote(_)));
        assert!(matches!(callout("> [link] não é callout"), Block::Quote(_)));
        assert!(matches!(callout("> [!] vazio"), Block::Quote(_)));
    }

    #[test]
    fn a_header_with_emphasis_in_the_body_keeps_the_emphasis() {
        let blocks = super::super::blocks(&super::super::prepare("> [!warning] **Cuidado**\n> e mais"));
        let Block::Callout { title, body, .. } = &blocks[0] else { panic!("{blocks:?}") };
        // The title is plain text even when the source wraps it in markers.
        assert_eq!(title, "Cuidado");
        assert_eq!(body[0].plain(), "e mais");
    }

    #[test]
    fn a_body_with_a_link_stays_a_callout() {
        let blocks = super::super::blocks(&super::super::prepare("> [!warning] Cuidado\n> veja [[Nota]]"));
        let Block::Callout { title, body, .. } = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(title, "Cuidado");
        let Block::Paragraph(spans) = &body[0] else { panic!("{body:?}") };
        assert!(spans.iter().any(|span| matches!(span, Span::Link { .. })), "{spans:?}");
    }

    #[test]
    fn a_nested_quote_inside_a_callout_survives() {
        let Block::Callout { body, .. } = callout("> [!note] T\n>\n> > dentro") else { panic!("not a callout") };
        assert!(matches!(body.first(), Some(Block::Quote(_))), "{body:?}");
    }
}
