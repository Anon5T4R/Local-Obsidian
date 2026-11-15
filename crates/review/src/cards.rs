//! Finding flashcards inside a note.
//!
//! A card is a callout plus a highlight - syntax the reading view already draws - so the note stays
//! readable anywhere and the cards degrade into plain prose when the review engine is not there.
//!
//! Port of the upstream `cards.ts`, including the two details that were bugs once:
//! nested callouts (`> > [!card]` inside a warning) are found, and a card written inside a code
//! fence is documentation, not a card.

/// The card types the vault recognises (`[!card]`, `[!flashcard]`, `[!pergunta]`).
const CARD_TYPES: [&str; 3] = ["card", "flashcard", "pergunta"];
/// The mnemonic types; only a `?` right after the type makes one reviewable.
const MNEMONIC_TYPES: [&str; 3] = ["mnemonic", "mnemonico", "mnemônico"];

/// What replaces the answer inside a cloze sentence.
pub const CLOZE_GAP: &str = "[ . ]";

/// How deep a callout may sit inside other callouts before we stop looking.
const MAX_NESTING: usize = 3;

/// What kind of card this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKind {
    /// Question in the callout's title, answer in its body.
    Qa,
    /// A sentence with one `==highlight==` hidden.
    Cloze,
    /// `> [!mnemonic]?` - a mnemonic the author marked as worth reviewing.
    Mnemonic,
}

/// One card found in a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedCard {
    /// Stable while the question does not change.
    pub id: String,
    pub kind: CardKind,
    /// Shown in the review panel: the question, or the sentence with its gap.
    pub q: String,
    /// The answer, or the term behind the gap.
    pub a: String,
    /// 0-based index of the gap inside a cloze sentence.
    pub cloze_index: Option<usize>,
}

/// Fowler/Noll/Vo 1a over UTF-16 code units, run forwards and backwards - a short, dependency-free
/// hash whose id only has to be stable across the desktop, the phone and this app.
pub fn card_id(relative_path: &str, question: &str, suffix: &str) -> String {
    let input = if suffix.is_empty() { format!("{relative_path}::{question}") } else { format!("{relative_path}::{question}::{suffix}") };
    let units: Vec<u16> = input.encode_utf16().collect();
    let mut forward: u32 = 0x811c_9dc5;
    for unit in &units {
        forward ^= u32::from(*unit);
        forward = forward.wrapping_mul(0x0100_0193);
    }
    let mut backward: u32 = 0x811c_9dc5;
    for unit in units.iter().rev() {
        backward ^= u32::from(*unit);
        backward = backward.wrapping_mul(0x0100_0193);
    }
    format!("{forward:08x}{backward:08x}")
}

/// The `==term==` occurrences of a text, in order.
pub fn cloze_parts(text: &str) -> Vec<String> {
    highlight_spans(text).into_iter().map(|(_, term)| term).collect()
}

/// Replaces the `hide_index`-th `==term==` with a gap and unwraps the others.
pub fn render_cloze(text: &str, hide_index: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (index, (start, term)) in highlight_spans(text).into_iter().enumerate() {
        out.push_str(&text[copied..start]);
        if index == hide_index {
            out.push_str(CLOZE_GAP);
        } else {
            out.push_str(&term);
        }
        // The span's `==` wrappers are dropped either way.
        copied = start + term.len() + 4;
    }
    out.push_str(&text[copied..]);
    out
}

/// `(start of the opening `==`, term)` for every `==term==`, in order. A term never crosses a line
/// break, and an empty `====` is not a term.
fn highlight_spans(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find("==") {
        let start = from + offset;
        let after = start + 2;
        let line_end = text[after..].find('\n').map_or(text.len(), |index| after + index);
        let Some(close) = text[after..line_end].find("==").map(|index| after + index) else {
            from = after;
            continue;
        };
        let term = &text[after..close];
        if !term.is_empty() {
            out.push((start, term.to_string()));
        }
        from = close + 2;
    }
    out
}

/// A callout as the note wrote it.
struct Callout {
    kind: String,
    fold: char,
    title: String,
    body: String,
}

/// The cards `content` declares, in document order.
pub fn extract_cards(relative_path: &str, content: &str) -> Vec<ExtractedCard> {
    let mut out = Vec::new();
    // A card written inside a code fence is documentation about cards, not a card.
    for callout in parse_callouts(&strip_fences(content), 0) {
        let is_card = CARD_TYPES.contains(&callout.kind.as_str());
        let is_mnemonic = MNEMONIC_TYPES.contains(&callout.kind.as_str());
        if !is_card && !is_mnemonic {
            continue;
        }
        if is_mnemonic && callout.fold != '?' {
            continue;
        }

        let gaps = cloze_parts(&callout.body);
        if is_card && !gaps.is_empty() {
            for (index, term) in gaps.iter().enumerate() {
                let rendered = render_cloze(&callout.body, index);
                let q = if callout.title.is_empty() { rendered } else { format!("{} - {rendered}", callout.title) };
                out.push(ExtractedCard {
                    id: card_id(relative_path, &callout.body, &format!("c{index}")),
                    kind: CardKind::Cloze,
                    q,
                    a: term.clone(),
                    cloze_index: Some(index),
                });
            }
            continue;
        }

        let question =
            if callout.title.is_empty() { callout.body.split('\n').next().unwrap_or_default().trim().to_string() } else { callout.title.clone() };
        if question.is_empty() {
            continue;
        }
        out.push(ExtractedCard {
            id: card_id(relative_path, &question, ""),
            kind: if is_mnemonic { CardKind::Mnemonic } else { CardKind::Qa },
            q: question,
            a: callout.body.clone(),
            cloze_index: None,
        });
    }
    out
}

/// Callout blocks in raw markdown, with their `> ` prefixes removed; nested ones are unwrapped.
fn parse_callouts(markdown: &str, depth: usize) -> Vec<Callout> {
    let mut out = Vec::new();
    let lines: Vec<&str> = markdown.split('\n').collect();
    let mut index = 0;
    while index < lines.len() {
        let Some((kind, fold, title)) = callout_head(lines[index]) else {
            index += 1;
            continue;
        };
        let mut body: Vec<&str> = Vec::new();
        let mut cursor = index + 1;
        while cursor < lines.len() && lines[cursor].trim_start().starts_with('>') {
            body.push(strip_quote_marker(lines[cursor]));
            cursor += 1;
        }
        let text = body.join("\n").trim().to_string();
        out.push(Callout { kind, fold, title, body: text.clone() });

        // A card written inside another callout - `> > [!card]` under a `> [!warning]` - is natural
        // to write and used to be dropped without a word. One less `>` makes the inner one top-level.
        if depth < MAX_NESTING && text.contains("[!") {
            out.extend(parse_callouts(&text, depth + 1));
        }
        index = cursor.max(index + 1);
    }
    out
}

/// `(kind, fold marker, title)` when the line opens a callout.
fn callout_head(line: &str) -> Option<(String, char, String)> {
    let rest = line.trim_start().strip_prefix('>')?.trim_start();
    let rest = rest.strip_prefix("[!")?;
    let close = rest.find(']')?;
    let kind = rest[..close].trim().to_lowercase();
    if kind.is_empty() || !kind.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let mut tail = &rest[close + 1..];
    let mut fold = ' ';
    if let Some(rest) = tail.strip_prefix(['-', '+', '?']) {
        fold = tail.chars().next().unwrap_or(' ');
        tail = rest;
    }
    Some((kind, fold, tail.trim().to_string()))
}

/// The line without its leading `> ` (and one optional space).
fn strip_quote_marker(line: &str) -> &str {
    let line = line.trim_start();
    let line = line.strip_prefix('>').unwrap_or(line);
    line.strip_prefix(' ').unwrap_or(line)
}

/// Blanks out fenced code so a card written as an example is not collected.
fn strip_fences(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut in_fence: Option<&str> = None;
    for line in markdown.split_inclusive('\n') {
        let trimmed = line.trim_start();
        match in_fence {
            None => {
                if trimmed.starts_with("```") {
                    in_fence = Some("```");
                    out.extend(line.chars().map(|c| if c == '\n' { '\n' } else { ' ' }));
                } else if trimmed.starts_with("~~~") {
                    in_fence = Some("~~~");
                    out.extend(line.chars().map(|c| if c == '\n' { '\n' } else { ' ' }));
                } else {
                    out.push_str(line);
                }
            }
            Some(fence) => {
                if trimmed.starts_with(fence) {
                    in_fence = None;
                }
                out.extend(line.chars().map(|c| if c == '\n' { '\n' } else { ' ' }));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_is_stable_and_tells_the_cards_apart() {
        assert_eq!(card_id("a.md", "Pergunta?", ""), card_id("a.md", "Pergunta?", ""));
        assert_ne!(card_id("a.md", "Pergunta?", ""), card_id("a.md", "Outra?", ""));
        assert_ne!(card_id("a.md", "P?", ""), card_id("b.md", "P?", ""));
        assert_ne!(card_id("a.md", "frase", "c0"), card_id("a.md", "frase", "c1"));
        assert_ne!(card_id("a.md", "abc", ""), card_id("a.md", "acb", ""));
    }

    #[test]
    fn the_id_counts_utf16_units_like_the_other_apps() {
        // The hash is over UTF-16 code units; the exact value is the contract with the phone.
        assert_eq!(card_id("a.md", "Pergunta?", "").len(), 16);
        assert_ne!(card_id("a.md", "café", ""), card_id("a.md", "cafe", ""));
    }

    #[test]
    fn the_highlights_are_listed_in_order() {
        let text = "A ==citrato sintase== condensa ==acetil-CoA==";
        assert_eq!(cloze_parts(text), ["citrato sintase", "acetil-CoA"]);
    }

    #[test]
    fn only_the_requested_gap_is_hidden() {
        let text = "A ==citrato sintase== condensa ==acetil-CoA==";
        assert_eq!(render_cloze(text, 0), format!("A {CLOZE_GAP} condensa acetil-CoA"));
        assert_eq!(render_cloze(text, 1), format!("A citrato sintase condensa {CLOZE_GAP}"));
    }

    #[test]
    fn a_question_and_answer_card_is_read() {
        let md = "> [!card]- Qual a tríade do qSOFA?\n> FR = 22 ou PAS = 100 ou Glasgow < 15.";
        let cards = extract_cards("Sepse.md", md);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, CardKind::Qa);
        assert_eq!(cards[0].q, "Qual a tríade do qSOFA?");
        assert!(cards[0].a.contains("FR = 22"));
    }

    #[test]
    fn the_aliases_of_the_card_type_are_accepted() {
        assert_eq!(extract_cards("a.md", "> [!flashcard]- P\n> R").len(), 1);
        assert_eq!(extract_cards("a.md", "> [!pergunta]- P\n> R").len(), 1);
    }

    #[test]
    fn a_cloze_sentence_gives_one_card_per_highlight() {
        let md = "> [!card] Ciclo de Krebs\n> A ==citrato sintase== condensa ==acetil-CoA==";
        let cards = extract_cards("Krebs.md", md);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].kind, CardKind::Cloze);
        assert!(cards[0].q.contains(CLOZE_GAP), "{}", cards[0].q);
        assert_eq!(cards[0].a, "citrato sintase");
        assert_eq!(cards[1].a, "acetil-CoA");
        assert_ne!(cards[0].id, cards[1].id);
    }

    #[test]
    fn a_mnemonic_only_counts_with_the_question_mark() {
        assert!(extract_cards("a.md", "> [!mnemonic] 5H e 5T\n> Hipovolemia.").is_empty());
        assert_eq!(extract_cards("a.md", "> [!mnemonic]? 5H e 5T\n> Hipovolemia.").len(), 1);
    }

    #[test]
    fn other_callouts_are_not_cards() {
        assert!(extract_cards("a.md", "> [!warning] Cuidado\n> texto").is_empty());
    }

    #[test]
    fn a_card_inside_a_code_fence_is_documentation() {
        let md = "```\n> [!card]- Exemplo\n> resposta\n```";
        assert!(extract_cards("a.md", md).is_empty());
    }

    #[test]
    fn a_card_nested_in_another_callout_is_found() {
        let md = "> [!warning] Cuidado\n> > [!card]- Aninhado?\n> > resposta";
        let cards = extract_cards("a.md", md);
        assert_eq!(cards.len(), 1, "{cards:?}");
        assert_eq!(cards[0].q, "Aninhado?");
    }

    #[test]
    fn a_card_without_a_title_takes_the_first_body_line() {
        let md = "> [!card]\n> Pergunta na primeira linha\n> resposta";
        let cards = extract_cards("a.md", md);
        assert_eq!(cards[0].q, "Pergunta na primeira linha");
    }

    #[test]
    fn two_cards_in_one_note_keep_their_order() {
        let md = "> [!card]- Um?\n> a\n\n> [!card]- Dois?\n> b";
        let cards = extract_cards("a.md", md);
        assert_eq!(cards.iter().map(|c| c.q.as_str()).collect::<Vec<_>>(), ["Um?", "Dois?"]);
    }
}
