//! The open vault, as the app holds it: the tree, the note list, the index, the note being edited
//! and everything that changes them. No egui here - the views read this state and hand back what
//! the user asked for.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use localobsidian_md::{embeds, preview};
use localobsidian_vault as vault;

/// Autosave waits this long after the last keystroke, so a burst of typing is one write.
pub const AUTOSAVE_AFTER: Duration = Duration::from_millis(800);

/// The vault and the note in front of the user.
#[derive(Default)]
pub struct VaultState {
    /// The vault folder, `None` before one is opened.
    pub root: Option<PathBuf>,
    /// The tree as the sidebar lists it (already ordered by [`Self::sort`]).
    pub tree: Vec<vault::Node>,
    /// Every note, for link resolution.
    pub files: Vec<vault::NoteFile>,
    /// Tags, aliases, backlinks and per-note metadata of everything that was read.
    pub index: vault::Index,
    /// The note being edited.
    pub active: Option<PathBuf>,
    /// Its text, as edited. Equals the file's content while [`Self::dirty`] is false.
    pub buffer: String,
    pub dirty: bool,
    /// The sidebar's filter box.
    pub filter: String,
    pub sort: vault::Sort,
    /// Reading view instead of the editor.
    pub preview_open: bool,
    /// A message for the status strip (a save, a rename's link count, an error).
    pub message: Option<String>,
    /// Bumped on every edit, so the reading view knows when to rebuild.
    pub revision: u64,
    /// The cached reading view of [`Self::revision`].
    preview_cache: Option<(u64, Vec<preview::Block>)>,
    contents: BTreeMap<PathBuf, String>,
    last_edit: Option<Instant>,
}

impl VaultState {
    /// Opens `root` as a vault: scans the tree, reads every note and builds the index.
    pub fn open(&mut self, root: PathBuf) -> io::Result<()> {
        self.root = Some(root);
        self.active = None;
        self.buffer.clear();
        self.dirty = false;
        self.filter.clear();
        self.reload()
    }

    /// Re-reads the vault from disk, keeping the open note (and unsaved text) in place.
    pub fn reload(&mut self) -> io::Result<()> {
        let Some(root) = self.root.clone() else { return Ok(()) };
        let scanned = vault::scan(&root)?;
        self.tree = vault::sorted(&scanned, self.sort);
        self.files = vault::flatten(&self.tree);

        self.contents = BTreeMap::new();
        for file in &self.files {
            match vault::read_note(&file.path) {
                Ok(text) => {
                    self.contents.insert(file.path.clone(), text);
                }
                Err(error) => log::warn!("vault: cannot read {}: {error}", file.path.display()),
            }
        }
        self.index = vault::index::build(&self.files, &self.contents);

        // A note that disappeared from disk (renamed elsewhere) closes; edits in flight survive
        // until the user picks a note again.
        if let Some(active) = &self.active
            && !self.files.iter().any(|file| &file.path == active)
        {
            self.active = None;
            self.buffer.clear();
            self.dirty = false;
        }
        Ok(())
    }

    /// Re-orders the sidebar without touching the disk.
    pub fn set_sort(&mut self, sort: vault::Sort) {
        self.sort = sort;
        self.tree = vault::sorted(&self.tree, sort);
    }

    /// The note to show first after opening a vault: the last one used, else the first of the tree.
    pub fn open_first_note(&mut self) {
        if self.active.is_some() {
            return;
        }
        if let Some(file) = self.files.first().map(|file| file.path.clone()) {
            self.open_note(&file);
        }
    }

    /// Puts `path` in the editor. Unsaved text of the note being left is written first, so a click in
    /// the sidebar can never lose a sentence.
    pub fn open_note(&mut self, path: &Path) {
        if self.active.as_deref() == Some(path) {
            return;
        }
        if self.dirty && self.save().is_err() {
            self.message = Some(crate::i18n::tr("could not save the note that was open").to_string());
        }
        match vault::read_note(path) {
            Ok(text) => {
                self.buffer = text;
                self.dirty = false;
                self.active = Some(path.to_path_buf());
                self.revision = self.revision.wrapping_add(1);
                self.preview_cache = None;
            }
            Err(error) => self.message = Some(format!("{}: {error}", path.display())),
        }
    }

    /// The editor's text changed.
    pub fn edit(&mut self, text: String) {
        if text == self.buffer {
            return;
        }
        self.buffer = text;
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
        self.last_edit = Some(Instant::now());
    }

    /// Writes the open note. A note nobody edited is not written again.
    pub fn save(&mut self) -> io::Result<()> {
        let Some(path) = self.active.clone() else { return Ok(()) };
        if !self.dirty {
            return Ok(());
        }
        vault::write_atomic(&path, &self.buffer)?;
        self.contents.insert(path.clone(), self.buffer.clone());
        self.dirty = false;
        self.last_edit = None;
        self.index = vault::index::build(&self.files, &self.contents);
        Ok(())
    }

    /// Writes the open note when the last keystroke is old enough. Returns whether it wrote.
    pub fn autosave(&mut self) -> bool {
        if !self.dirty {
            return false;
        }
        let due = self.last_edit.is_some_and(|at| at.elapsed() >= AUTOSAVE_AFTER);
        if due {
            match self.save() {
                Ok(()) => self.message = Some(crate::i18n::tr("saved").to_string()),
                Err(error) => self.message = Some(format!("{}: {error}", crate::i18n::tr("could not save the note"))),
            }
        }
        due
    }

    /// Creates `name.md` in the vault root and opens it.
    pub fn create_note(&mut self, name: &str) -> io::Result<PathBuf> {
        let root = self.root.clone().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no vault is open"))?;
        let folder = self.active.as_ref().and_then(|path| path.parent().map(Path::to_path_buf)).unwrap_or(root);
        let path = vault::create_note(&folder, name)?;
        self.reload()?;
        self.open_note(&path);
        Ok(path)
    }

    /// Renames the open note and rewrites every `[[link]]` that pointed at it in the vault.
    /// Returns how many links changed.
    pub fn rename_active(&mut self, new_name: &str) -> io::Result<usize> {
        let Some(path) = self.active.clone() else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no note is open"));
        };
        let old_name = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default().to_string();

        // The note's own unsaved text goes to disk first: a rename moves the file, and the buffer
        // must follow the file it belongs to.
        self.save()?;
        let renamed = vault::rename_note(&path, new_name)?;

        let mut written = 0;
        for file in &self.files {
            let Some(text) = self.contents.get(&file.path) else { continue };
            let (rewritten, count) = localobsidian_md::rewrite::rewrite(text, &old_name, &stem(&renamed));
            if count == 0 {
                continue;
            }
            vault::write_atomic(&file.path, &rewritten)?;
            self.contents.insert(file.path.clone(), rewritten);
            written += count;
        }
        self.active = Some(renamed);
        self.reload()?;
        Ok(written)
    }

    /// The note a raw wikilink target points at, from the open note.
    pub fn resolve(&self, target: &str) -> Option<&vault::NoteFile> {
        vault::resolve_note(&self.files, target, self.active.as_deref(), Some(&self.index.aliases))
    }

    /// The markdown of the note a raw wikilink target points at (what an `![[embed]]` needs).
    pub fn content_for(&self, target: &str) -> Option<String> {
        let file = self.resolve(target)?;
        if Some(&file.path) == self.active.as_ref() && !self.buffer.is_empty() {
            return Some(self.buffer.clone());
        }
        self.contents.get(&file.path).cloned()
    }

    /// Rebuilds the reading view when the text changed (cheap when it did not).
    pub fn refresh_preview(&mut self) {
        let stale = self.preview_cache.as_ref().is_none_or(|(revision, _)| *revision != self.revision);
        if stale {
            let expanded = embeds::expand(&self.buffer, &|target| self.content_for(target));
            let prepared = preview::prepare(&expanded);
            self.preview_cache = Some((self.revision, preview::blocks(&prepared)));
        }
    }

    /// The reading view as last built by [`Self::refresh_preview`].
    pub fn preview(&self) -> &[preview::Block] {
        self.preview_cache.as_ref().map(|(_, blocks)| blocks.as_slice()).unwrap_or_default()
    }

    /// The tree as the sidebar shows it, with the filter applied.
    pub fn filtered_tree(&self) -> Vec<vault::Node> {
        if self.filter.trim().is_empty() {
            return self.tree.clone();
        }
        filter_tree(&self.tree, &self.index, self.filter.trim())
    }

    /// How many notes the current filter keeps.
    pub fn filtered_count(&self) -> usize {
        self.filtered_tree().iter().map(count_notes).sum()
    }
}

fn stem(path: &Path) -> String {
    path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default().to_string()
}

fn filter_tree(nodes: &[vault::Node], index: &vault::Index, query: &str) -> Vec<vault::Node> {
    let mut out = Vec::new();
    for node in nodes {
        match node.kind {
            vault::Kind::File => {
                let info = index.info_of(&node.path);
                let view = vault::Filterable { name: &node.name, tags: &info.tags, aliases: &info.aliases };
                if vault::matches_filter(&view, query) {
                    out.push(node.clone());
                }
            }
            vault::Kind::Directory => {
                let mut kept = node.clone();
                kept.children = filter_tree(&node.children, index, query);
                if !kept.children.is_empty() {
                    out.push(kept);
                }
            }
        }
    }
    out
}

fn count_notes(node: &vault::Node) -> usize {
    match node.kind {
        vault::Kind::File => 1,
        vault::Kind::Directory => node.children.iter().map(count_notes).sum(),
    }
}
