//! ` ```query ` blocks: the note's own lens on the vault.
//!
//! A fence whose language is `query` is not code to show - it is a question to answer:
//!
//! ```text
//! tag: #tipo-patologia
//! path: 2. Medicina
//! sort: titulo
//! limit: 20
//! ```
//!
//! The reading view runs it against the notes the app already indexed, so the answer is the vault
//! as it is now, not a list somebody pasted once. Lines the parser does not understand are reported
//! back instead of being ignored: a query that silently matches everything is worse than an error.

use crate::index::Index;
use crate::tree::NoteFile;

/// How the answers are ordered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Order {
    /// By name, the default: the same list every time.
    #[default]
    Name,
    /// Most recently modified first.
    Modified,
}

/// What a query block asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// Every tag must be on the note (an AND, like filters in the search panel).
    pub tags: Vec<String>,
    /// A piece of the note's relative path.
    pub path: Option<String>,
    /// A piece of the note's name.
    pub file: Option<String>,
    pub sort: Order,
    /// The most notes to show.
    pub limit: Option<usize>,
    /// Lines that make no sense, as written.
    pub mistakes: Vec<String>,
}

impl Query {
    /// Whether the block asks for anything at all.
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty() && self.path.is_none() && self.file.is_none()
    }
}

/// Reads a block's source. Never fails: what it cannot use comes back in [`Query::mistakes`].
pub fn parse(source: &str) -> Query {
    let mut query = Query::default();
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') && !line.contains(':') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            query.mistakes.push(line.to_string());
            continue;
        };
        let key = key.trim().to_lowercase();
        let value = value.trim();
        match key.as_str() {
            "tag" | "tags" => {
                for tag in value.split([',', ' ']).map(str::trim).filter(|tag| !tag.is_empty()) {
                    query.tags.push(tag.trim_start_matches('#').to_lowercase());
                }
            }
            "path" => query.path = Some(value.to_string()),
            "file" | "name" => query.file = Some(value.to_string()),
            "sort" => match value.to_lowercase().as_str() {
                "titulo" | "título" | "title" | "nome" | "name" => query.sort = Order::Name,
                "modificado" | "modified" | "mtime" | "edited" => query.sort = Order::Modified,
                other => query.mistakes.push(format!("{key}: {other}")),
            },
            "limit" => match value.parse::<usize>() {
                Ok(limit) if limit > 0 => query.limit = Some(limit),
                _ => query.mistakes.push(format!("{key}: {value}")),
            },
            _ => query.mistakes.push(line.to_string()),
        }
    }
    query
}

/// The notes `query` selects, in the order it asked for.
pub fn run<'a>(files: &'a [NoteFile], index: &Index, query: &Query) -> Vec<&'a NoteFile> {
    let mut selected: Vec<&NoteFile> = files
        .iter()
        .filter(|note| {
            query.tags.iter().all(|tag| index.tags.get(tag).is_some_and(|names| names.iter().any(|name| name.eq_ignore_ascii_case(&note.name))))
        })
        .filter(|note| query.path.as_ref().is_none_or(|path| note.relative_path.to_lowercase().contains(&path.to_lowercase())))
        .filter(|note| query.file.as_ref().is_none_or(|file| note.name.to_lowercase().contains(&file.to_lowercase())))
        .collect();

    match query.sort {
        Order::Name => selected.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.relative_path.cmp(&b.relative_path))),
        Order::Modified => selected.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))),
    }
    if let Some(limit) = query.limit {
        selected.truncate(limit);
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn vault(notes: &[(&str, &str, u64)]) -> (Vec<NoteFile>, Index) {
        let files: Vec<NoteFile> = notes
            .iter()
            .map(|(relative, _, mtime_ms)| NoteFile {
                name: relative.rsplit('/').next().unwrap_or(relative).trim_end_matches(".md").to_string(),
                path: PathBuf::from(format!("/v/{relative}")),
                relative_path: (*relative).to_string(),
                mtime_ms: *mtime_ms,
            })
            .collect();
        let contents: BTreeMap<PathBuf, String> =
            notes.iter().map(|(relative, text, _)| (PathBuf::from(format!("/v/{relative}")), (*text).to_string())).collect();
        let index = crate::index::build(&files, &contents);
        (files, index)
    }

    fn names(notes: &[&NoteFile]) -> Vec<String> {
        notes.iter().map(|note| note.name.clone()).collect()
    }

    #[test]
    fn a_tag_selects_the_notes_that_carry_it() {
        let (files, index) = vault(&[
            ("A.md", "---\ntags: [tipo-patologia]\n---\n\n# A\n", 0),
            ("B.md", "---\ntags: [tipo-indice]\n---\n\n# B\n", 0),
            ("C.md", "nada\n", 0),
        ]);
        let query = parse("tag: #tipo-patologia");
        assert_eq!(names(&run(&files, &index, &query)), ["A"]);
    }

    #[test]
    fn two_tags_are_an_and_not_an_or() {
        let (files, index) = vault(&[("A.md", "---\ntags: [uti, sepse]\n---\n\n# A\n", 0), ("B.md", "---\ntags: [uti]\n---\n\n# B\n", 0)]);
        assert_eq!(names(&run(&files, &index, &parse("tags: #uti #sepse"))), ["A"]);
    }

    #[test]
    fn path_file_and_sort_shape_the_list() {
        let (files, index) = vault(&[("2. Medicina/Beta.md", "nada", 10), ("2. Medicina/Alfa.md", "nada", 30), ("1. Dashboard/Gama.md", "nada", 20)]);
        assert_eq!(names(&run(&files, &index, &parse("path: 2. Medicina"))), ["Alfa", "Beta"]);
        assert_eq!(names(&run(&files, &index, &parse("file: gam"))), ["Gama"]);
        assert_eq!(names(&run(&files, &index, &parse("path: medicina\nsort: modificado"))), ["Alfa", "Beta"]);
        assert_eq!(names(&run(&files, &index, &parse("limit: 1"))), ["Alfa"]);
    }

    #[test]
    fn an_empty_query_selects_every_note_in_name_order() {
        let (files, index) = vault(&[("b.md", "nada", 0), ("A.md", "nada", 0)]);
        let query = parse("");
        assert!(query.is_empty());
        assert_eq!(names(&run(&files, &index, &query)), ["A", "b"]);
    }

    #[test]
    fn lines_the_parser_cannot_use_come_back_as_mistakes() {
        let query = parse("tag: #uti\ncor: azul\nsort: alfabetico\nlimit: zero\nlinha solta");
        assert_eq!(query.tags, ["uti"], "what is valid is still used");
        assert_eq!(query.mistakes, ["cor: azul", "sort: alfabetico", "limit: zero", "linha solta"]);
    }
}
