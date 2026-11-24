//! Turning a `[[wikilink]]` target into a file of this vault.
//!
//! A target with a `/` is matched against the note's path inside the vault; anything else is
//! matched against the file name. When several notes share a name the one closest to the linking
//! note wins, otherwise which twin you get would depend on scan order. A real file name always
//! beats an alias.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use localnotebook_md::links;

use crate::tree::NoteFile;

/// The note `target` points at, `from` being the note that holds the link.
pub fn resolve_note<'a>(
    files: &'a [NoteFile],
    target: &str,
    from: Option<&Path>,
    aliases: Option<&BTreeMap<String, PathBuf>>,
) -> Option<&'a NoteFile> {
    let wanted = links::norm(target);
    if wanted.is_empty() {
        return None;
    }

    let matches: Vec<&NoteFile> = if wanted.contains('/') {
        files
            .iter()
            .filter(|file| {
                let relative = links::norm(&file.relative_path);
                relative == wanted || relative.ends_with(&format!("/{wanted}"))
            })
            .collect()
    } else {
        files.iter().filter(|file| file.name.to_lowercase() == wanted).collect()
    };

    if matches.is_empty() {
        let path = aliases?.get(&wanted)?;
        return files.iter().find(|file| &file.path == path);
    }
    if matches.len() == 1 {
        return matches.first().copied();
    }
    let Some(from) = from else { return matches.first().copied() };

    let mut best = matches[0];
    let mut best_depth = 0;
    for candidate in matches {
        let depth = shared_depth(&candidate.path, from);
        if depth > best_depth {
            best = candidate;
            best_depth = depth;
        }
    }
    Some(best)
}

/// Number of leading folders two notes share - Obsidian's "closest note wins", and the reason the
/// answer does not depend on scan order.
fn shared_depth(a: &Path, b: &Path) -> usize {
    let left = parents(a);
    let right = parents(b);
    left.iter().zip(right.iter()).take_while(|(one, other)| one == other).count()
}

fn parents(path: &Path) -> Vec<String> {
    let normalized = links::norm(&path.to_string_lossy());
    let mut parts: Vec<String> = normalized.split('/').map(str::to_string).collect();
    // The last part is the file itself, not a folder.
    parts.pop();
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(relative_path: &str) -> NoteFile {
        let name = relative_path.rsplit('/').next().unwrap_or(relative_path).trim_end_matches(".md").to_string();
        NoteFile { name, path: PathBuf::from(format!("C:/vault/{relative_path}")), relative_path: relative_path.to_string(), mtime_ms: 0 }
    }

    fn files() -> Vec<NoteFile> {
        vec![
            note("Sepse.md"),
            note("Patologias/Cirrose Hepática.md"),
            note("Clínica Médica/Cirrose Hepática.md"),
            note("Clínica Médica/Notas/Anemia.md"),
        ]
    }

    #[test]
    fn resolves_by_name_ignoring_case() {
        assert_eq!(resolve_note(&files(), "sepse", None, None).map(|f| f.relative_path.as_str()), Some("Sepse.md"));
    }

    #[test]
    fn resolves_by_folder_path() {
        assert_eq!(
            resolve_note(&files(), "Patologias/Cirrose Hepática", None, None).map(|f| f.relative_path.as_str()),
            Some("Patologias/Cirrose Hepática.md")
        );
    }

    #[test]
    fn accepts_the_extension_in_the_target() {
        assert_eq!(resolve_note(&files(), "Sepse.md", None, None).map(|f| f.name.as_str()), Some("Sepse"));
    }

    #[test]
    fn an_unknown_note_resolves_to_nothing() {
        assert!(resolve_note(&files(), "Não Existe", None, None).is_none());
    }

    #[test]
    fn the_closest_note_wins_when_two_share_a_name() {
        let from = Path::new("C:/vault/Clínica Médica/Notas/Anemia.md");
        assert_eq!(
            resolve_note(&files(), "Cirrose Hepática", Some(from), None).map(|f| f.relative_path.as_str()),
            Some("Clínica Médica/Cirrose Hepática.md")
        );
    }

    #[test]
    fn without_a_source_note_the_first_match_wins() {
        assert_eq!(resolve_note(&files(), "Cirrose Hepática", None, None).map(|f| f.relative_path.as_str()), Some("Patologias/Cirrose Hepática.md"));
    }

    #[test]
    fn an_alias_resolves_when_no_file_name_matches() {
        let mut aliases = BTreeMap::new();
        aliases.insert("sepse grave".to_string(), PathBuf::from("C:/vault/Sepse.md"));
        assert_eq!(resolve_note(&files(), "Sepse Grave", None, Some(&aliases)).map(|f| f.name.as_str()), Some("Sepse"));
    }

    #[test]
    fn a_file_name_beats_an_alias() {
        let mut aliases = BTreeMap::new();
        aliases.insert("sepsе".to_string(), PathBuf::from("C:/vault/Outra.md"));
        // Same name as a real note: the file wins even though the alias points elsewhere.
        assert_eq!(resolve_note(&files(), "Sepse", None, Some(&aliases)).map(|f| f.name.as_str()), Some("Sepse"));
    }

    #[test]
    fn an_empty_target_resolves_to_nothing() {
        assert!(resolve_note(&files(), "   ", None, None).is_none());
    }
}
