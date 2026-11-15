//! What the vault holds: the folder tree, and the flat note list built from it.

use std::path::{Path, PathBuf};

/// A tree entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
}

/// One entry of the sidebar tree, with everything already read that the view needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The file name without `.md` (the note name), or the folder name.
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    /// Last write time in milliseconds since the epoch; `0` for folders.
    pub mtime_ms: u64,
    pub children: Vec<Node>,
}

impl Node {
    pub fn is_directory(&self) -> bool {
        self.kind == Kind::Directory
    }
}

/// A note of the vault, as the flat list and the link resolver see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteFile {
    /// The note name: the file name without `.md`.
    pub name: String,
    pub path: PathBuf,
    /// Path inside the vault, with `/` separators and the `.md` extension (`Pasta/Nota.md`).
    pub relative_path: String,
    pub mtime_ms: u64,
}

/// How one level of the tree is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    /// A→Z, case-insensitive (the upstream `localeCompare` by name).
    #[default]
    Name,
    /// Z→A.
    NameDesc,
    /// Most recently modified first.
    Modified,
}

/// Reads `root` into a tree.
///
/// Hidden entries (a leading `.`) are skipped: that is what keeps `.openobsidian/`, `.git/` and the
/// atomic-write temp files out of the sidebar. A folder that cannot be read yields no children
/// instead of failing the scan, so one unreadable directory never blanks the vault.
///
/// Only `.md` files are listed. Binaries (PDF, DOCX, EPUB, ODT) stay out until the app can open
/// them; listing a file that cannot be opened is worse than not listing it.
pub fn scan(root: &Path) -> std::io::Result<Vec<Node>> {
    let mut nodes = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(nodes),
        Err(error) => return Err(error),
    };

    let mut entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            let children = scan(&path).unwrap_or_else(|error| {
                log::warn!("vault: cannot read {}: {error}", path.display());
                Vec::new()
            });
            nodes.push(Node { name, path, kind: Kind::Directory, mtime_ms: 0, children });
        } else if file_type.is_file() && is_note_name(&name) {
            let mtime_ms = mtime_ms(&path);
            let stem = name.trim_end_matches(".md").to_string();
            nodes.push(Node { name: stem, path, kind: Kind::File, mtime_ms, children: Vec::new() });
        }
    }
    Ok(nodes)
}

/// The tree ordered for display: loose notes first, folders last, each group by `sort`.
///
/// Files-first is the upstream decision and it holds up: a folder's indented children never butt
/// up against a sibling file of the same level.
pub fn sorted(nodes: &[Node], sort: Sort) -> Vec<Node> {
    let mut level: Vec<Node> = nodes.to_vec();
    level.sort_by(|a, b| {
        if a.kind != b.kind {
            return if a.kind == Kind::File { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
        }
        match sort {
            Sort::Name => compare_names(a, b),
            Sort::NameDesc => compare_names(b, a),
            Sort::Modified => b.mtime_ms.cmp(&a.mtime_ms),
        }
    });
    for node in &mut level {
        if node.kind == Kind::Directory {
            node.children = sorted(&node.children, sort);
        }
    }
    level
}

/// Every note in the tree, in tree order.
pub fn flatten(nodes: &[Node]) -> Vec<NoteFile> {
    let mut files = Vec::new();
    walk(nodes, "", &mut files);
    files
}

fn walk(nodes: &[Node], prefix: &str, files: &mut Vec<NoteFile>) {
    for node in nodes {
        match node.kind {
            Kind::File => files.push(NoteFile {
                name: node.name.clone(),
                path: node.path.clone(),
                relative_path: format!("{prefix}{}.md", node.name),
                mtime_ms: node.mtime_ms,
            }),
            Kind::Directory => {
                let inner = format!("{prefix}{}/", node.name);
                walk(&node.children, &inner, files);
            }
        }
    }
}

/// A file name the tree lists: a Markdown note, and not a dotfile (checked by the caller).
fn is_note_name(name: &str) -> bool {
    name.len() > 3 && name.ends_with(".md")
}

fn mtime_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Case-insensitive A→Z, with the case-sensitive order as the tie-break so the result is stable.
fn compare_names(a: &Node, b: &Node) -> std::cmp::Ordering {
    a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, mtime_ms: u64) -> Node {
        Node { name: name.into(), path: PathBuf::from(format!("/v/{name}.md")), kind: Kind::File, mtime_ms, children: Vec::new() }
    }

    fn folder(name: &str, children: Vec<Node>) -> Node {
        Node { name: name.into(), path: PathBuf::from(format!("/v/{name}")), kind: Kind::Directory, mtime_ms: 0, children }
    }

    fn names(nodes: &[Node]) -> Vec<String> {
        nodes.iter().map(|node| node.name.clone()).collect()
    }

    /// The upstream `treeSort.test.ts` level, deliberately shuffled.
    fn level() -> Vec<Node> {
        vec![folder("Projects", Vec::new()), file("banana", 100), folder("Archive", Vec::new()), file("apple", 300)]
    }

    #[test]
    fn files_come_first_then_folders_by_name() {
        assert_eq!(names(&sorted(&level(), Sort::Name)), ["apple", "banana", "Archive", "Projects"]);
    }

    #[test]
    fn descending_keeps_files_first() {
        assert_eq!(names(&sorted(&level(), Sort::NameDesc)), ["banana", "apple", "Projects", "Archive"]);
    }

    #[test]
    fn modified_puts_the_most_recent_note_first_and_keeps_folders_last() {
        assert_eq!(names(&sorted(&level(), Sort::Modified)), ["apple", "banana", "Projects", "Archive"]);
    }

    #[test]
    fn sorting_does_not_mutate_the_input() {
        let original = level();
        let _ = sorted(&original, Sort::Name);
        assert_eq!(names(&original), names(&level()));
    }

    #[test]
    fn sorting_reaches_into_folders() {
        let tree = vec![folder("Pasta", vec![file("zeta", 0), file("alfa", 0)])];
        let sorted = sorted(&tree, Sort::Name);
        assert_eq!(names(&sorted[0].children), ["alfa", "zeta"]);
    }

    #[test]
    fn flatten_keeps_only_notes_and_builds_the_relative_path() {
        let tree = vec![file("root", 0), folder("folder", vec![file("nested", 0), folder("deeper", vec![file("fundo", 0)])])];
        let files = flatten(&tree);
        let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
        assert_eq!(paths, ["root.md", "folder/nested.md", "folder/deeper/fundo.md"]);
    }

    #[test]
    fn only_markdown_files_are_notes() {
        assert!(is_note_name("nota.md"));
        assert!(!is_note_name("doc.pdf"));
        assert!(!is_note_name("md"));
        assert!(!is_note_name(".md"));
    }
}
