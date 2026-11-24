//! The vault index: what every note declares and what it links to.
//!
//! One pass over the notes' contents produces four answers the UI needs at typing speed:
//!
//! - **backlinks** - `[[target]]` → the notes that link it (the raw target is the key, anchor
//!   included, so a section link and a whole-note link are counted like the author wrote them);
//! - **tags** - `#tag` and the frontmatter's `tags:`, with every parent of a nested tag;
//! - **frontmatter** - each note's fields, parsed once;
//! - **aliases** - an alias → the note that declares it, first declaration wins so a duplicated
//!   alias stays predictable.
//!
//! Notes are keyed by their path; the names in `backlinks`/`tags` are the note names, which is what
//! the sidebar shows.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use localnotebook_md::{frontmatter, links, tags};

use crate::tree::NoteFile;

/// The metadata of one note, kept per path so the sidebar filter never re-parses a note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteInfo {
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub title: Option<String>,
}

/// Everything the app reads out of the notes' text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Index {
    /// Wikilink target as written → the names of the notes that link it.
    pub backlinks: BTreeMap<String, Vec<String>>,
    /// Tag (lowercase, parents included) → the names of the notes that carry it.
    pub tags: BTreeMap<String, Vec<String>>,
    /// Note path → its fields.
    pub frontmatter: BTreeMap<PathBuf, frontmatter::Frontmatter>,
    /// Alias (lowercase) → the path of the note that declares it.
    pub aliases: BTreeMap<String, PathBuf>,
    /// Note path → what the sidebar filter matches against.
    pub notes: BTreeMap<PathBuf, NoteInfo>,
    /// Note path → the paths it links to (resolved, in document order, no self, no dangling).
    pub links: BTreeMap<PathBuf, Vec<PathBuf>>,
    /// Note path → the paths that link it (the reverse of [`Self::links`]).
    pub linked_from: BTreeMap<PathBuf, Vec<PathBuf>>,
}

/// Builds the index from the notes and their contents. A note missing from `contents` contributes
/// nothing: the index describes what was read, not what was hoped for.
pub fn build(files: &[NoteFile], contents: &BTreeMap<PathBuf, String>) -> Index {
    let mut index = Index::default();

    for file in files {
        let Some(text) = contents.get(&file.path) else { continue };
        let parsed = frontmatter::parse(text);
        let declared_tags = parsed.as_ref().map(frontmatter::Frontmatter::tags).unwrap_or_default();
        let declared_aliases = parsed.as_ref().map(frontmatter::Frontmatter::aliases).unwrap_or_default();

        let mut note_tags = tags::extract(text);
        note_tags.extend(declared_tags);
        let note_tags = tags::expand_hierarchy(&note_tags);
        for tag in &note_tags {
            push_once(index.tags.entry(tag.clone()).or_default(), &file.name);
        }

        for target in links::extract(text) {
            push_once(index.backlinks.entry(target).or_default(), &file.name);
        }

        for alias in &declared_aliases {
            let key = alias.trim().to_lowercase();
            if !key.is_empty() {
                index.aliases.entry(key).or_insert_with(|| file.path.clone());
            }
        }

        if let Some(fields) = parsed {
            index.frontmatter.insert(file.path.clone(), fields);
        }
        index.notes.insert(file.path.clone(), NoteInfo { tags: note_tags, aliases: declared_aliases, title: None });
    }

    // The resolved edges, in a second pass because an alias can be declared in a note that was
    // read later: every note's outgoing links, and the reverse in the same walk, so the connection
    // map never re-reads a note's text to find out what is next to it.
    for file in files {
        let Some(text) = contents.get(&file.path) else { continue };
        for target in links::extract(text) {
            let name = links::parse_target(&target).note;
            let Some(other) = crate::resolve_note(files, name, Some(&file.path), Some(&index.aliases)) else { continue };
            if other.path == file.path {
                continue;
            }
            let outgoing = index.links.entry(file.path.clone()).or_default();
            if !outgoing.iter().any(|existing| existing == &other.path) {
                outgoing.push(other.path.clone());
            }
            let incoming = index.linked_from.entry(other.path.clone()).or_default();
            if !incoming.iter().any(|existing| existing == &file.path) {
                incoming.push(file.path.clone());
            }
        }
    }

    index
}

fn push_once(list: &mut Vec<String>, name: &str) {
    if !list.iter().any(|existing| existing == name) {
        list.push(name.to_string());
    }
}

impl Index {
    /// The notes that link `target` (as written), if any.
    pub fn backlinks_of(&self, target: &str) -> &[String] {
        self.backlinks.get(target).map(Vec::as_slice).unwrap_or_default()
    }

    /// What the sidebar filter matches against, for `path`.
    pub fn info_of(&self, path: &Path) -> NoteInfo {
        self.notes.get(path).cloned().unwrap_or_default()
    }

    /// How many notes declare `tag` (parents included).
    pub fn tag_count(&self, tag: &str) -> usize {
        self.tags.get(tag).map(Vec::len).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(name: &str) -> NoteFile {
        NoteFile { name: name.to_string(), path: PathBuf::from(format!("/v/{name}.md")), relative_path: format!("{name}.md"), mtime_ms: 0 }
    }

    fn index_of(notes: &[(&str, &str)]) -> Index {
        let files: Vec<NoteFile> = notes.iter().map(|(name, _)| note(name)).collect();
        let contents: BTreeMap<PathBuf, String> =
            notes.iter().map(|(name, text)| (PathBuf::from(format!("/v/{name}.md")), text.to_string())).collect();
        build(&files, &contents)
    }

    #[test]
    fn links_point_back_at_their_source_note() {
        let index = index_of(&[("Alfa", "veja [[Beta]] e [[Beta|x]]"), ("Beta", "nada")]);
        assert_eq!(index.backlinks_of("Beta"), ["Alfa"]);
    }

    #[test]
    fn the_resolved_edges_are_the_ones_the_map_walks() {
        let index =
            index_of(&[("Alfa", "[[Beta]] e [[Beta#Seção]] e [[Fantasma]]"), ("Beta", "veja [[Alfa]] e ele mesmo [[Beta]]"), ("Gama", "nada")]);
        let alfa = PathBuf::from("/v/Alfa.md");
        let beta = PathBuf::from("/v/Beta.md");
        let gama = PathBuf::from("/v/Gama.md");

        // Outgoing: resolved, deduplicated (a section link is the same note), no dangling, no self.
        assert_eq!(index.links.get(&alfa), Some(&vec![beta.clone()]));
        assert_eq!(index.links.get(&beta), Some(&vec![alfa.clone()]), "a link back is one edge, not two");
        assert_eq!(index.links.get(&gama), None, "a note with no links has no entry");

        // Incoming is the same edge seen from the other end.
        assert_eq!(index.linked_from.get(&beta), Some(&vec![alfa.clone()]));
        assert_eq!(index.linked_from.get(&alfa), Some(&vec![beta.clone()]));
    }

    #[test]
    fn an_alias_in_another_note_is_an_edge() {
        let index = index_of(&[("Alfa", "[[Lacunas]]"), ("Painel", "---\naliases: [Lacunas]\n---\n\nnada")]);
        assert_eq!(index.links.get(&PathBuf::from("/v/Alfa.md")), Some(&vec![PathBuf::from("/v/Painel.md")]));
    }

    #[test]
    fn tags_carry_their_parents() {
        let index = index_of(&[("Alfa", "#sistema/cardio")]);
        assert_eq!(index.tag_count("sistema"), 1);
        assert_eq!(index.tag_count("sistema/cardio"), 1);
    }

    #[test]
    fn frontmatter_tags_and_aliases_join_the_index() {
        let index = index_of(&[("Alfa", "---\ntags: [utz]\naliases: [SCA]\n---\ncorpo")]);
        assert_eq!(index.tag_count("utz"), 1);
        assert_eq!(index.aliases.get("sca"), Some(&PathBuf::from("/v/Alfa.md")));
        assert!(index.frontmatter.contains_key(Path::new("/v/Alfa.md")));
    }

    #[test]
    fn the_first_declaration_of_an_alias_wins() {
        let index = index_of(&[("Alfa", "---\naliases: [X]\n---\n"), ("Beta", "---\naliases: [X]\n---\n")]);
        assert_eq!(index.aliases.get("x"), Some(&PathBuf::from("/v/Alfa.md")));
    }

    #[test]
    fn a_note_whose_content_was_not_read_contributes_nothing() {
        let files = vec![note("Alfa")];
        let index = build(&files, &BTreeMap::new());
        assert!(index.tags.is_empty() && index.backlinks.is_empty() && index.notes.is_empty());
    }

    #[test]
    fn info_of_gives_the_filter_the_tags_and_aliases() {
        let index = index_of(&[("Alfa", "---\naliases: [SCA]\n---\n#cardio")]);
        let info = index.info_of(Path::new("/v/Alfa.md"));
        assert!(info.tags.contains(&"cardio".to_string()));
        assert_eq!(info.aliases, ["SCA"]);
    }
}
