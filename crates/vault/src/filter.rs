//! Matching for the sidebar's filter box.
//!
//! A filter on a tree, meant to be instant while you type: it matches the note's name, its tags and
//! its aliases, never its text (full text search is a different feature, and a worse one inside a
//! tree - the upstream kept that split deliberately).
//!
//! `#foo` restricts the match to tags. Any other term matches name, tag or alias. A tag matches by
//! prefix, so `cardio` finds `#cardio/isquemia`. Several terms narrow the result - all must match,
//! which is what `#uti, cardio` naturally reads as.

/// What the filter sees of a note.
pub struct Filterable<'a> {
    pub name: &'a str,
    pub tags: &'a [String],
    pub aliases: &'a [String],
}

/// Whether `note` passes `query`.
pub fn matches_filter(note: &Filterable<'_>, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    // A name can hold spaces ("teste de sprint 2"), so a whole-query match wins before the input is
    // read as several terms.
    if matches_term(note, &query) {
        return true;
    }
    let terms: Vec<&str> = query.split([',', ' ']).filter(|term| !term.is_empty()).collect();
    if terms.len() < 2 {
        return false;
    }
    terms.iter().all(|term| matches_term(note, term))
}

fn matches_term(note: &Filterable<'_>, term: &str) -> bool {
    let tag_hit = |needle: &str| note.tags.iter().any(|tag| tag.to_lowercase().contains(needle));

    if let Some(needle) = term.strip_prefix('#') {
        return if needle.is_empty() { !note.tags.is_empty() } else { tag_hit(needle) };
    }
    note.name.to_lowercase().contains(term) || tag_hit(term) || note.aliases.iter().any(|alias| alias.to_lowercase().contains(term))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A note that owns its strings, so a test can ask for a [`Filterable`] as often as it likes.
    struct Note {
        name: String,
        tags: Vec<String>,
        aliases: Vec<String>,
    }

    impl Note {
        fn filterable(&self) -> Filterable<'_> {
            Filterable { name: &self.name, tags: &self.tags, aliases: &self.aliases }
        }

        fn without_tags(&self) -> Note {
            Note { name: self.name.clone(), tags: Vec::new(), aliases: self.aliases.clone() }
        }
    }

    /// The upstream `sidebarFilter.test.ts` note.
    fn sca() -> Note {
        Note {
            name: "Síndrome Coronariana Aguda (SCA)".to_string(),
            tags: vec!["sistema/cardio".to_string(), "sistema".to_string(), "emergencia".to_string()],
            aliases: vec!["IAM".to_string(), "infarto".to_string()],
        }
    }

    #[test]
    fn an_empty_query_matches_everything() {
        assert!(matches_filter(&sca().filterable(), "   "));
    }

    #[test]
    fn matches_by_name_tag_and_alias() {
        let note = sca();
        for query in ["coronariana", "emergencia", "cardio", "iam"] {
            assert!(matches_filter(&note.filterable(), query), "{query}");
        }
    }

    #[test]
    fn rejects_what_is_nowhere() {
        assert!(!matches_filter(&sca().filterable(), "pediatria"));
    }

    #[test]
    fn a_hash_restricts_the_match_to_tags() {
        let note = sca();
        assert!(matches_filter(&note.filterable(), "#emergencia"));
        assert!(!matches_filter(&note.filterable(), "#iam"));
        assert!(!matches_filter(&note.filterable(), "#coronariana"));
    }

    #[test]
    fn a_lone_hash_keeps_every_tagged_note() {
        assert!(matches_filter(&sca().filterable(), "#"));
        assert!(!matches_filter(&sca().without_tags().filterable(), "#"));
    }

    #[test]
    fn case_is_ignored_on_both_sides() {
        assert!(matches_filter(&sca().filterable(), "SCA"));
    }

    #[test]
    fn several_terms_all_have_to_match() {
        let note = sca();
        assert!(matches_filter(&note.filterable(), "#emergencia, cardio"));
        assert!(!matches_filter(&note.filterable(), "#emergencia, pediatria"));
        assert!(matches_filter(&note.filterable(), "iam cardio"));
        assert!(!matches_filter(&note.filterable(), "#uti cardio"));
    }

    #[test]
    fn a_name_with_spaces_matches_whole() {
        assert!(matches_filter(&sca().filterable(), "coronariana aguda"));
    }
}
