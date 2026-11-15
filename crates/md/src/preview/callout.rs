//! `> [!warning] Careful` - the callout syntax the Markdown note ecosystem shares (Obsidian,
//! GitHub-flavoured Markdown), plus this app's own `[!embed]` head.
//!
//! A quote stays a quote: only the one whose first line is `[!kind]` becomes a callout. The type is
//! kept as written (lowercase), the title falls back to the type's name, and a `+`/`-` right after
//! the type is the fold marker of the syntax - the reading view shows the body either way.
//!
//! The header line is only taken when it is plain text. A title with emphasis in it stays a plain
//! quote, because dropping the inline formatting to gain a coloured box would lose what the author
//! wrote.

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
fn take_header(inner: &mut Vec<Block>) -> Option<(String, String)> {
    let Block::Paragraph(spans) = inner.first()? else { return None };
    // One plain run: a header with emphasis stays a quote (see the module note).
    let [Span::Text(text)] = spans.as_slice() else { return None };

    // `prepare` put a hard break after the header, so the body of the callout may follow it in the
    // same paragraph: `[!note]\ncorpo`.
    let (head, rest) = match text.split_once('\n') {
        Some((head, rest)) => (head, Some(rest)),
        None => (text.as_str(), None),
    };

    let head = head.trim_start().strip_prefix("[!")?;
    let close = head.find(']')?;
    let kind = head[..close].trim().to_lowercase();
    if !is_kind(&kind) {
        return None;
    }

    let mut title = head[close + 1..].trim().trim_end_matches('\\').trim();
    if let Some(unfolded) = title.strip_prefix(['+', '-']) {
        title = unfolded.trim();
    }
    let title = if title.is_empty() { title_case(&kind) } else { title.to_string() };

    match rest.map(str::trim).filter(|rest| !rest.is_empty()) {
        Some(body) => inner[0] = Block::Paragraph(vec![Span::Text(body.to_string())]),
        None => {
            inner.remove(0);
        }
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
    fn a_header_with_emphasis_stays_a_quote() {
        assert!(matches!(callout("> [!warning] **Cuidado**"), Block::Quote(_)));
    }

    #[test]
    fn a_nested_quote_inside_a_callout_survives() {
        let Block::Callout { body, .. } = callout("> [!note] T\n>\n> > dentro") else { panic!("not a callout") };
        assert!(matches!(body.first(), Some(Block::Quote(_))), "{body:?}");
    }
}
