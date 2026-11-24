//! What is wrong with the vault: links that point at nothing, notes nothing reaches, and names that
//! are not unique.
//!
//! The ledger the vault accumulates over months - a note deleted, a name typed with a `ç` and
//! without it - is invisible while reading one note at a time. This module answers the three
//! questions a reader asks when the vault starts to feel unreliable, from the index the app already
//! built: no extra pass over the notes' text.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use localnotebook_md::links;

use crate::index::Index;
use crate::tree::NoteFile;

/// A `[[target]]` no note answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenLink {
    /// The note the link names, as written (section stripped), sorted and deduplicated.
    pub target: String,
    /// The notes that write it, by name.
    pub from: Vec<String>,
}

/// Two notes with the same name in different folders: a `[[link]]` to that name lands on whichever
/// the resolver picks, and the reader never sees the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateName {
    pub name: String,
    /// Every note with that name, by relative path.
    pub files: Vec<String>,
}

/// The three findings, each already sorted for showing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    pub broken_links: Vec<BrokenLink>,
    /// Notes with no link in and no link out, by relative path.
    pub orphans: Vec<String>,
    pub duplicates: Vec<DuplicateName>,
}

impl Diagnostics {
    /// Whether the vault has anything to say at all.
    pub fn is_clean(&self) -> bool {
        self.broken_links.is_empty() && self.orphans.is_empty() && self.duplicates.is_empty()
    }
}

/// Diagnoses `files` from their index.
pub fn diagnose(files: &[NoteFile], index: &Index) -> Diagnostics {
    let mut broken: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (target, sources) in &index.backlinks {
        let name = links::parse_target(target).note;
        if name.trim().is_empty() {
            continue;
        }
        // A link to a section of a note that does not exist is the same broken link.
        if crate::resolve_note(files, name, None, Some(&index.aliases)).is_none() {
            broken.entry(name.to_string()).or_default().extend(sources.iter().cloned());
        }
    }
    let broken_links: Vec<BrokenLink> = broken.into_iter().map(|(target, from)| BrokenLink { target, from: from.into_iter().collect() }).collect();

    let is_linked = |path: &Path| index.links.contains_key(path) || index.linked_from.contains_key(path);
    let orphans: Vec<String> = {
        let mut orphans: Vec<String> = files.iter().filter(|note| !is_linked(&note.path)).map(|note| note.relative_path.clone()).collect();
        orphans.sort();
        orphans
    };

    let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for note in files {
        by_name.entry(note.name.to_lowercase()).or_default().push(note.relative_path.clone());
    }
    let duplicates: Vec<DuplicateName> = by_name
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(_, mut files)| {
            files.sort();
            // The name as the first file writes it, so the panel shows `Sepse`, not `sepse`.
            let shown = files[0].rsplit('/').next().unwrap_or(&files[0]).trim_end_matches(".md").to_string();
            DuplicateName { name: shown, files }
        })
        .collect();

    Diagnostics { broken_links, orphans, duplicates }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn vault(notes: &[(&str, &str)]) -> (Vec<NoteFile>, Index) {
        let files: Vec<NoteFile> = notes
            .iter()
            .map(|(relative, _)| NoteFile {
                name: relative.rsplit('/').next().unwrap_or(relative).trim_end_matches(".md").to_string(),
                path: PathBuf::from(format!("/v/{relative}")),
                relative_path: (*relative).to_string(),
                mtime_ms: 0,
            })
            .collect();
        let contents: BTreeMap<PathBuf, String> =
            notes.iter().map(|(relative, text)| (PathBuf::from(format!("/v/{relative}")), (*text).to_string())).collect();
        let index = crate::index::build(&files, &contents);
        (files, index)
    }

    #[test]
    fn a_link_to_a_note_that_does_not_exist_is_reported_with_who_wrote_it() {
        let (files, index) = vault(&[("A.md", "[[Fantasma]] e [[Fantasma#Seção]]"), ("B.md", "[[Outra coisa]]"), ("C.md", "nada")]);
        let diagnostics = diagnose(&files, &index);
        let targets: Vec<&str> = diagnostics.broken_links.iter().map(|link| link.target.as_str()).collect();
        assert_eq!(targets, ["Fantasma", "Outra coisa"]);
        assert_eq!(diagnostics.broken_links[0].from, ["A"]);
        assert!(!diagnostics.broken_links[0].from.contains(&"C".to_string()));
    }

    #[test]
    fn a_note_nothing_reaches_and_that_reaches_nothing_is_an_orphan() {
        let (files, index) = vault(&[("Ligada.md", "[[Outra]]"), ("Outra.md", "nada"), ("So.md", "nada")]);
        let diagnostics = diagnose(&files, &index);
        assert_eq!(diagnostics.orphans, ["So.md"]);
    }

    #[test]
    fn two_notes_with_the_same_name_are_reported_together() {
        let (files, index) = vault(&[("Pasta/Sepse.md", "nada"), ("Outra/Sepse.md", "nada"), ("Unica.md", "nada")]);
        let diagnostics = diagnose(&files, &index);
        assert_eq!(diagnostics.duplicates.len(), 1);
        assert_eq!(diagnostics.duplicates[0].name, "Sepse");
        assert_eq!(diagnostics.duplicates[0].files, ["Outra/Sepse.md", "Pasta/Sepse.md"]);
    }

    #[test]
    fn a_healthy_vault_has_nothing_to_report() {
        let (files, index) = vault(&[("A.md", "[[B]]"), ("B.md", "[[A]]")]);
        assert!(diagnose(&files, &index).is_clean());
    }
}
