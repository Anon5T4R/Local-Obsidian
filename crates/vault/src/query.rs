//! ` ```query ` blocks: the note's own lens on the vault.
//!
//! A fence whose language is `query` is not code to show - it is a question to answer, in the same
//! grammar the Android app reads (one dialect, on purpose):
//!
//! ```text
//! tag: #tipo-patologia
//! path: 2. Medicina
//! tipo: patologia          # any frontmatter field
//! has: aliases             # the field only needs to exist
//! sort: titulo desc
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
    /// By name (`titulo`/`title`), the default: the same list every time.
    #[default]
    Name,
    /// Most recently modified first (`modificado`/`modified`).
    Modified,
    /// The path inside the vault (`caminho`/`path`).
    Path,
}

/// What a query block asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// Every tag must be on the note (an AND, like the search panel's filters).
    pub tags: Vec<String>,
    /// Pieces of the note's relative path (any of them matches).
    pub paths: Vec<String>,
    /// Frontmatter field → accepted values, lowercase; any value matches.
    pub fields: Vec<(String, Vec<String>)>,
    /// Fields that only need to exist.
    pub has: Vec<String>,
    pub order: Order,
    /// Ordering from Z to A (or newest first for dates).
    pub descending: bool,
    /// The most notes to show.
    pub limit: Option<usize>,
    /// Lines that make no sense, as written.
    pub mistakes: Vec<String>,
}

impl Query {
    /// Whether the block asks for anything at all.
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty() && self.paths.is_empty() && self.fields.is_empty() && self.has.is_empty()
    }
}

/// Splits a value list the way the grammar does: commas, with quotes trimmed; tags also accept
/// spaces, because a tag never has one.
fn values(raw: &str, spaces: bool) -> Vec<String> {
    let split: Vec<&str> = if spaces { raw.split([',', ' ']).collect() } else { raw.split(',').collect() };
    split.into_iter().map(|value| value.trim().trim_matches(['"', '\'']).to_string()).filter(|value| !value.is_empty()).collect()
}

/// Reads a block's source. Never fails: what it cannot use comes back in [`Query::mistakes`].
pub fn parse(source: &str) -> Query {
    let mut query = Query::default();
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            query.mistakes.push(line.to_string());
            continue;
        };
        let key = key.trim().to_lowercase();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            query.mistakes.push(line.to_string());
            continue;
        }
        match key.as_str() {
            "tag" | "tags" => {
                query.tags.extend(values(value, true).into_iter().map(|tag| tag.trim_start_matches('#').to_lowercase()));
            }
            "path" | "pasta" => query.paths.extend(values(value, false).into_iter().map(|path| path.to_lowercase())),
            "has" | "tem" => query.has.extend(values(value, true).into_iter().map(|field| field.to_lowercase())),
            "sort" | "ordenar" => {
                let mut words: Vec<&str> = value.split_whitespace().collect();
                let descending = words.last().is_some_and(|word| matches!(word.to_lowercase().as_str(), "desc" | "descending" | "inverso"));
                if descending {
                    words.pop();
                }
                let name = words.join(" ").to_lowercase();
                match name.as_str() {
                    "titulo" | "título" | "title" | "nome" | "name" => query.order = Order::Name,
                    "modificado" | "modified" | "editado" => query.order = Order::Modified,
                    // No birth time in the vault: "created" means "last written", which is the closest
                    // the folder can answer.
                    "criado" | "created" => query.order = Order::Modified,
                    "caminho" | "path" => query.order = Order::Path,
                    _ => {
                        query.mistakes.push(line.to_string());
                        continue;
                    }
                }
                query.descending = descending;
            }
            "limit" | "limite" => match value.parse::<usize>() {
                Ok(limit) if limit > 0 => query.limit = Some(limit),
                _ => query.mistakes.push(line.to_string()),
            },
            // Anything else is a frontmatter field, which is what makes `tipo: patologia` work.
            _ => query.fields.push((key, values(value, false).into_iter().map(|value| value.to_lowercase()).collect())),
        }
    }
    query
}

/// The notes `query` selects, in the order it asked for.
pub fn run<'a>(files: &'a [NoteFile], index: &Index, query: &Query) -> Vec<&'a NoteFile> {
    let fields_of = |note: &NoteFile| -> Vec<(String, String)> {
        index.frontmatter.get(&note.path).map(localnotebook_md::frontmatter::Frontmatter::pairs).unwrap_or_default()
    };
    let mut selected: Vec<&NoteFile> = files
        .iter()
        .filter(|note| {
            query.tags.iter().all(|tag| index.tags.get(tag).is_some_and(|names| names.iter().any(|name| name.eq_ignore_ascii_case(&note.name))))
        })
        .filter(|note| query.paths.is_empty() || query.paths.iter().any(|path| note.relative_path.to_lowercase().contains(path)))
        .filter(|note| {
            if query.fields.is_empty() && query.has.is_empty() {
                return true;
            }
            let fields = fields_of(note);
            let has = |key: &str| fields.iter().any(|(field, _)| field.eq_ignore_ascii_case(key));
            query.has.iter().all(|field| has(field))
                && query.fields.iter().all(|(key, wanted)| {
                    fields.iter().filter(|(field, _)| field.eq_ignore_ascii_case(key)).any(|(_, value)| {
                        let value = value.to_lowercase();
                        wanted.iter().any(|want| value.split(',').any(|part| part.trim() == want))
                    })
                })
        })
        .collect();

    let order = |note: &NoteFile| match query.order {
        Order::Name => note.name.to_lowercase(),
        Order::Path => note.relative_path.to_lowercase(),
        Order::Modified => String::new(),
    };
    match query.order {
        Order::Modified => selected.sort_by(|a, b| {
            let by_time = if query.descending { a.mtime_ms.cmp(&b.mtime_ms) } else { b.mtime_ms.cmp(&a.mtime_ms) };
            by_time.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        }),
        _ if query.descending => selected.sort_by(|a, b| order(b).cmp(&order(a)).then_with(|| b.relative_path.cmp(&a.relative_path))),
        _ => selected.sort_by(|a, b| order(a).cmp(&order(b)).then_with(|| a.relative_path.cmp(&b.relative_path))),
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
        assert_eq!(names(&run(&files, &index, &parse("tag: #tipo-patologia"))), ["A"]);
    }

    #[test]
    fn two_tags_are_an_and_not_an_or() {
        let (files, index) = vault(&[("A.md", "---\ntags: [uti, sepse]\n---\n\n# A\n", 0), ("B.md", "---\ntags: [uti]\n---\n\n# B\n", 0)]);
        assert_eq!(names(&run(&files, &index, &parse("tags: #uti, #sepse"))), ["A"]);
    }

    #[test]
    fn a_frontmatter_field_is_a_filter_and_so_is_has() {
        let (files, index) = vault(&[
            ("A.md", "---\ntipo: patologia\naliases: [Sepse]\n---\n\n# A\n", 0),
            ("B.md", "---\ntipo: sistema\n---\n\n# B\n", 0),
            ("C.md", "nada\n", 0),
        ]);
        assert_eq!(names(&run(&files, &index, &parse("tipo: patologia"))), ["A"]);
        assert_eq!(names(&run(&files, &index, &parse("has: aliases"))), ["A"]);
        assert_eq!(names(&run(&files, &index, &parse("tipo: patologia, sistema"))), ["A", "B"], "any value of the field");
    }

    #[test]
    fn path_order_and_limit_shape_the_list() {
        let (files, index) = vault(&[("2. Medicina/Beta.md", "nada", 10), ("2. Medicina/Alfa.md", "nada", 30), ("1. Dashboard/Gama.md", "nada", 20)]);
        assert_eq!(names(&run(&files, &index, &parse("path: 2. Medicina"))), ["Alfa", "Beta"]);
        assert_eq!(names(&run(&files, &index, &parse("sort: modificado"))), ["Alfa", "Gama", "Beta"]);
        assert_eq!(names(&run(&files, &index, &parse("sort: titulo desc"))), ["Gama", "Beta", "Alfa"]);
        assert_eq!(names(&run(&files, &index, &parse("limit: 1"))), ["Alfa"]);
        assert_eq!(names(&run(&files, &index, &parse("sort: caminho"))), ["Gama", "Alfa", "Beta"]);
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
        let query = parse("tag: #uti\nsort: alfabetico\nlimit: zero\nlinha solta\n: vazio");
        assert_eq!(query.tags, ["uti"], "what is valid is still used");
        assert_eq!(query.mistakes, ["sort: alfabetico", "limit: zero", "linha solta", ": vazio"]);
    }
}
