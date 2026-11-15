//! The search panel's grammar and matching, ported from the upstream `searchQuery.ts`.
//!
//! The vault's central use case is "search by symptom, land on the conduct fast", which is a
//! faceted search - hence the `tag:`, `path:` and `file:` operators, `"exact phrases"` and `-term`
//! exclusions. Matching is `toLowerCase` plus `includes`: **no accent folding and no stemming**, as
//! the upstream decided (limitation #1 there is a deliberate trade, not a bug).

use regex::Regex;

/// A parsed query: what the box on screen means.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// Free text; all terms must appear in the note.
    pub terms: Vec<String>,
    /// `"exact phrase"`; all phrases must appear.
    pub phrases: Vec<String>,
    /// `-term` / `-"phrase"`: none may appear.
    pub excluded: Vec<String>,
    /// `tag:x` (the `#` is optional, children count).
    pub tags: Vec<String>,
    /// `path:x` against the note's path inside the vault.
    pub paths: Vec<String>,
    /// `file:x` against the note's name and its aliases.
    pub files: Vec<String>,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.phrases.is_empty()
            && self.excluded.is_empty()
            && self.tags.is_empty()
            && self.paths.is_empty()
            && self.files.is_empty()
    }

    /// What a note's text is searched for - what gets highlighted in the results.
    pub fn needles(&self) -> Vec<String> {
        self.phrases.iter().chain(self.terms.iter()).cloned().collect()
    }
}

/// One matching line of a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMatch {
    /// 1-based, as the search panel shows it.
    pub line: usize,
    /// The line, trimmed.
    pub text: String,
}

/// The parts of a note the search reads.
pub struct Searchable<'a> {
    pub name: &'a str,
    pub relative_path: &'a str,
    pub content: &'a str,
    pub tags: &'a [String],
    pub aliases: &'a [String],
}

/// Parses the box's text.
pub fn parse_query(raw: &str) -> Query {
    let mut query = Query::default();

    for token in tokenize(raw) {
        let negated = token.starts_with('-') && token.len() > 1;
        let body = if negated { &token[1..] } else { token.as_str() };

        if let Some((field, value)) = field_of(body) {
            let value = unquote(value).to_lowercase();
            if value.is_empty() {
                continue;
            }
            match field.as_str() {
                "tag" => query.tags.push(value.trim_start_matches('#').to_string()),
                "path" => query.paths.push(value),
                "file" => query.files.push(value),
                _ => {}
            }
            continue;
        }

        let is_phrase = body.starts_with('"');
        let value = unquote(body).trim().to_lowercase();
        if value.is_empty() {
            continue;
        }
        if negated {
            query.excluded.push(value);
        } else if is_phrase {
            query.phrases.push(value);
        } else {
            query.terms.push(value);
        }
    }
    query
}

/// `(field, rest)` for `tag:`, `path:` and `file:` (case-insensitive).
fn field_of(token: &str) -> Option<(String, &str)> {
    let (field, value) = token.split_once(':')?;
    let field = field.to_lowercase();
    matches!(field.as_str(), "tag" | "path" | "file").then_some((field, value))
}

/// Splits on whitespace but keeps `"quoted phrases"` (and `-"negated"` ones) whole, like the
/// upstream `-?"[^"]*"?|\S+`. An unclosed quote runs to the end of the box.
fn tokenize(raw: &str) -> Vec<String> {
    let chars: Vec<char> = raw.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index].is_whitespace() {
            index += 1;
            continue;
        }
        let start = index;
        let mut cursor = index;
        if chars[cursor] == '-' {
            cursor += 1;
        }
        if cursor < chars.len() && chars[cursor] == '"' {
            cursor += 1;
            while cursor < chars.len() && chars[cursor] != '"' {
                cursor += 1;
            }
            if cursor < chars.len() {
                cursor += 1; // the closing quote
            }
        } else {
            while cursor < chars.len() && !chars[cursor].is_whitespace() {
                cursor += 1;
            }
        }
        tokens.push(chars[start..cursor].iter().collect());
        index = cursor;
    }
    tokens
}

/// The token without the quotes a phrase may carry.
fn unquote(text: &str) -> &str {
    text.trim_start_matches('"').trim_end_matches('"')
}

/// Whether `note` satisfies `query`, and the lines that match.
///
/// `None` = not a result. `Some(vec![])` = a result with no matching line, which is what a filter
/// alone (`tag:cardio`) means. With `pattern`, the free text is that regular expression instead of
/// the literal needles.
pub fn search(note: &Searchable<'_>, query: &Query) -> Option<Vec<LineMatch>> {
    search_with(note, query, None)
}

/// [`search`] with an optional regular expression for the free text.
pub fn search_with(note: &Searchable<'_>, query: &Query, pattern: Option<&Regex>) -> Option<Vec<LineMatch>> {
    let content = note.content.to_lowercase();

    // A tag filter matches the tag itself or any of its children.
    for tag in &query.tags {
        let hit = note.tags.iter().any(|own| own == tag || own.starts_with(&format!("{tag}/")));
        if !hit {
            return None;
        }
    }

    let relative = note.relative_path.replace('\\', "/").to_lowercase();
    if !query.paths.iter().all(|path| relative.contains(path.as_str())) {
        return None;
    }
    // `file:` matches the name or any of its aliases.
    let names: Vec<String> = std::iter::once(note.name.to_lowercase()).chain(note.aliases.iter().map(|alias| alias.to_lowercase())).collect();
    if !query.files.iter().all(|needle| names.iter().any(|name| name.contains(needle.as_str()))) {
        return None;
    }

    let needles = query.needles();
    if pattern.is_none() && !needles.iter().all(|needle| content.contains(needle.as_str())) {
        return None;
    }
    if query.excluded.iter().any(|excluded| content.contains(excluded.as_str())) {
        return None;
    }

    let mut matches = Vec::new();
    let regex_mode = pattern.is_some();
    if regex_mode || !needles.is_empty() {
        for (index, line) in note.content.split('\n').enumerate() {
            let hit = match pattern {
                Some(pattern) => pattern.is_match(line),
                None => {
                    let lower = line.to_lowercase();
                    needles.iter().any(|needle| lower.contains(needle.as_str()))
                }
            };
            if hit {
                matches.push(LineMatch { line: index + 1, text: line.trim().to_string() });
            }
        }
        // Regex mode has no whole-note pre-check, so no lines means no result.
        if regex_mode && matches.is_empty() {
            return None;
        }
    }
    Some(matches)
}

/// How well a note answers the query, so the results are not buried under alphabetical order.
///
/// A hit in the title means the note *is* about the term, a hit in a heading means a section is, a
/// hit in the body is just a mention. Counts grow with diminishing returns: a long note must not
/// win on sheer repetition over a note actually about the subject.
pub fn score(note: &Searchable<'_>, query: &Query, matches: &[LineMatch]) -> f32 {
    let needles = query.needles();
    if needles.is_empty() {
        return matches.len() as f32;
    }

    let name = note.name.to_lowercase();
    let aliases: Vec<String> = note.aliases.iter().map(|alias| alias.to_lowercase()).collect();
    let mut score = 0.0_f32;

    for needle in &needles {
        if name == *needle {
            score += 100.0;
        } else if name.contains(needle.as_str()) {
            score += 40.0;
        }
        if aliases.iter().any(|alias| alias == needle) {
            score += 80.0;
        } else if aliases.iter().any(|alias| alias.contains(needle.as_str())) {
            score += 30.0;
        }
    }

    let mut headings = 0_u32;
    let mut body = 0_u32;
    for hit in matches {
        let text = hit.text.to_lowercase();
        if !needles.iter().any(|needle| text.contains(needle.as_str())) {
            continue;
        }
        // A heading hit outranks a body hit: headings are the note's own outline.
        if is_heading(&hit.text) {
            headings += 1;
        } else {
            body += 1;
        }
    }
    score += (headings * 12).min(48) as f32;
    score += (1.0 + body as f32).log2() * 6.0;
    score
}

fn is_heading(line: &str) -> bool {
    line.starts_with('#')
}

/// Compiles the free text as a case-insensitive regular expression. An invalid pattern is `None`,
/// which the panel shows as "that pattern is not valid" instead of failing.
pub fn compile_pattern(raw: &str) -> Option<Regex> {
    if raw.trim().is_empty() {
        return None;
    }
    Regex::new(&format!("(?i){raw}")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The upstream `searchQuery.test.ts` note.
    struct Fixture {
        name: String,
        relative_path: String,
        content: String,
        tags: Vec<String>,
        aliases: Vec<String>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                name: "Sepse".into(),
                relative_path: "Patologias/Sepse.md".into(),
                content: "# Sepse\n\nLactato alto na admissão.\nRepetir lactato em 2h.\nAntibiótico em 1h.".into(),
                tags: vec!["uti".into(), "sistema/cardio".into(), "sistema".into()],
                aliases: Vec::new(),
            }
        }

        fn searchable(&self) -> Searchable<'_> {
            Searchable { name: &self.name, relative_path: &self.relative_path, content: &self.content, tags: &self.tags, aliases: &self.aliases }
        }

        fn with(self, name: &str, content: &str) -> Self {
            Self { name: name.into(), content: content.into(), ..self }
        }
    }

    fn find(query: &str) -> Option<Vec<LineMatch>> {
        let fixture = Fixture::new();
        search(&fixture.searchable(), &parse_query(query))
    }

    #[test]
    fn plain_terms_and_phrases_split_apart() {
        assert_eq!(parse_query("lactato sepse").terms, ["lactato", "sepse"]);
        let query = parse_query("\"choque séptico\" uti");
        assert_eq!(query.phrases, ["choque séptico"]);
        assert_eq!(query.terms, ["uti"]);
    }

    #[test]
    fn exclusions_come_off_the_term() {
        assert_eq!(parse_query("sepse -pediatria").excluded, ["pediatria"]);
        assert_eq!(parse_query("-\"choque séptico\"").excluded, ["choque séptico"]);
    }

    #[test]
    fn the_field_operators_are_case_insensitive_and_take_a_hash() {
        let query = parse_query("TAG:UTI path:emergencia file:sepse");
        assert_eq!(
            (query.tags.clone(), query.paths.clone(), query.files.clone()),
            (vec!["uti".to_string()], vec!["emergencia".to_string()], vec!["sepse".to_string()])
        );
        assert_eq!(parse_query("tag:#uti").tags, ["uti"]);
    }

    #[test]
    fn an_operator_with_no_value_is_ignored() {
        assert!(parse_query("tag:").is_empty());
        assert!(parse_query("   ").is_empty());
    }

    #[test]
    fn an_unclosed_quote_is_still_a_phrase() {
        assert_eq!(parse_query("\"sem fim").phrases, ["sem fim"]);
    }

    #[test]
    fn every_occurrence_comes_back_with_its_line() {
        let matches = find("lactato").expect("a result");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0], LineMatch { line: 3, text: "Lactato alto na admissão.".into() });
    }

    #[test]
    fn every_term_must_appear_somewhere_in_the_note() {
        assert!(find("lactato inexistente").is_none());
        assert!(find("lactato antibiótico").is_some(), "terms on different lines still match");
    }

    #[test]
    fn a_tag_filter_keeps_the_note_but_its_children_count() {
        assert_eq!(find("tag:uti"), Some(Vec::new()));
        assert!(find("tag:pediatria").is_none());
        let mut fixture = Fixture::new();
        fixture.tags = vec!["sistema/cardio".into()];
        assert_eq!(search(&fixture.searchable(), &parse_query("tag:sistema")), Some(Vec::new()));
    }

    #[test]
    fn path_and_file_filters_match_by_substring() {
        assert_eq!(find("path:patologias"), Some(Vec::new()));
        assert!(find("path:emergencia").is_none());
        assert_eq!(find("file:seps"), Some(Vec::new()));
        assert!(find("file:outro").is_none());
    }

    #[test]
    fn a_filter_combines_with_a_term() {
        assert_eq!(find("tag:uti lactato").map(|m| m.len()), Some(2));
        assert!(find("tag:pediatria lactato").is_none());
    }

    #[test]
    fn an_excluded_term_drops_the_note() {
        assert!(find("lactato -antibiótico").is_none());
    }

    #[test]
    fn a_phrase_has_to_be_exact() {
        assert_eq!(find("\"repetir lactato\"").map(|m| m.len()), Some(1));
        assert!(find("\"lactato repetir\"").is_none());
    }

    #[test]
    fn a_regular_expression_replaces_the_free_text() {
        let fixture = Fixture::new();
        let query = parse_query("lactat.");
        let pattern = Regex::new("(?i)lactat.").expect("valid");
        assert_eq!(search_with(&fixture.searchable(), &query, Some(&pattern)).map(|m| m.len()), Some(2));
        let pattern = Regex::new("(?i)xyz+").expect("valid");
        assert!(search_with(&fixture.searchable(), &parse_query("xyz+"), Some(&pattern)).is_none());
    }

    #[test]
    fn a_title_match_outranks_a_mention() {
        let titled = Fixture::new().with("Lactato", "Texto qualquer com lactato.");
        let mention = Fixture::new().with("Sepse", "Repetir lactato em 2h.");
        let query = parse_query("lactato");
        let score_of = |fixture: &Fixture| {
            let matches = search(&fixture.searchable(), &query).unwrap_or_default();
            score(&fixture.searchable(), &query, &matches)
        };
        assert!(score_of(&titled) > score_of(&mention));
    }

    #[test]
    fn a_heading_hit_outranks_a_body_hit() {
        let heading = Fixture::new().with("A", "## Lactato\n\ntexto");
        let body = Fixture::new().with("B", "texto\n\nfalando de lactato");
        let query = parse_query("lactato");
        let score_of = |fixture: &Fixture| {
            let matches = search(&fixture.searchable(), &query).unwrap_or_default();
            score(&fixture.searchable(), &query, &matches)
        };
        assert!(score_of(&heading) > score_of(&body));
    }

    #[test]
    fn an_exact_alias_counts_almost_like_a_title() {
        let mut by_alias = Fixture::new().with("Síndrome Coronariana", "nada aqui de i a m: IAM");
        by_alias.aliases = vec!["IAM".into()];
        let mention = Fixture::new().with("Outra", "menciona IAM uma vez");
        let query = parse_query("iam");
        let score_of = |fixture: &Fixture| {
            let matches = search(&fixture.searchable(), &query).unwrap_or_default();
            score(&fixture.searchable(), &query, &matches)
        };
        assert!(score_of(&by_alias) > score_of(&mention));
    }

    #[test]
    fn repetition_does_not_beat_relevance() {
        let repeated = Fixture::new().with("Longa", &vec!["lactato"; 200].join("\n"));
        let titled = Fixture::new().with("Lactato", "uma menção de lactato");
        let query = parse_query("lactato");
        let score_of = |fixture: &Fixture| {
            let matches = search(&fixture.searchable(), &query).unwrap_or_default();
            score(&fixture.searchable(), &query, &matches)
        };
        assert!(score_of(&titled) > score_of(&repeated));
    }

    #[test]
    fn a_query_without_free_text_scores_by_the_match_count() {
        let fixture = Fixture::new();
        let query = parse_query("tag:uti");
        assert_eq!(score(&fixture.searchable(), &query, &[]), 0.0);
    }
}
