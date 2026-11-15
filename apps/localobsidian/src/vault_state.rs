//! The open vault, as the app holds it: the tree, the note list, the index, the note being edited
//! and everything that changes them. No egui here - the views read this state and hand back what
//! the user asked for.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use localobsidian_md::{embeds, links, preview};
use localobsidian_vault as vault;

/// Autosave waits this long after the last keystroke, so a burst of typing is one write.
pub const AUTOSAVE_AFTER: Duration = Duration::from_millis(800);

/// Which pane the note is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// The Markdown source.
    #[default]
    Edit,
    /// The reading view.
    Read,
    /// Both, side by side.
    Split,
}

impl ViewMode {
    /// What the View menu and the toggle button show as the value.
    pub fn key(self) -> &'static str {
        match self {
            ViewMode::Edit => "edit",
            ViewMode::Read => "read",
            ViewMode::Split => "split",
        }
    }

    /// The mode a settings file asked for; anything unknown falls back to the editor.
    pub fn from_key(key: &str) -> Self {
        match key {
            "read" => ViewMode::Read,
            "split" => ViewMode::Split,
            _ => ViewMode::Edit,
        }
    }

    /// The mode the toggle button cycles into.
    pub fn next(self) -> Self {
        match self {
            ViewMode::Edit => ViewMode::Read,
            ViewMode::Read => ViewMode::Split,
            ViewMode::Split => ViewMode::Edit,
        }
    }

    /// Whether the reading view is on screen (alone or beside the editor).
    pub fn shows_reading(self) -> bool {
        matches!(self, ViewMode::Read | ViewMode::Split)
    }
}

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
    /// Which pane the note is shown in.
    pub view: ViewMode,
    /// A message for the status strip (a save, a rename's link count, an error).
    pub message: Option<String>,
    /// Bumped on every edit, so the reading view knows when to rebuild.
    pub revision: u64,
    /// The cached reading view of [`Self::revision`].
    preview_cache: Option<(u64, Vec<preview::Block>)>,
    /// The filtered tree, and the inputs it was built from (the filter and the tree's generation).
    filtered_cache: Option<(u64, String, Vec<vault::Node>)>,
    /// Bumped whenever the tree changes, so the filtered view knows when to rebuild.
    tree_generation: u64,
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
        self.tree_generation = self.tree_generation.wrapping_add(1);

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
        self.tree_generation = self.tree_generation.wrapping_add(1);
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

    /// Marks the buffer as edited: dirty, a new revision for the reading view, autosave due.
    pub fn touch(&mut self) {
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

    /// The file an image destination points at, inside the vault.
    ///
    /// A note writes `![[foto.png]]` or `![alt](_attachments/foto.png)`; both land here. The lookup
    /// follows the vault's own convention - the note's folder first, then the vault root, then
    /// `_attachments/` in either - and never leaves the vault or touches the network.
    pub fn attachment_path(&self, destination: &str) -> Option<PathBuf> {
        let root = self.root.as_ref()?;
        let target = links::from_url(destination).unwrap_or_else(|| destination.to_string());
        let target = target.trim();
        if target.is_empty() || target.starts_with("http://") || target.starts_with("https://") || target.starts_with("data:") {
            return None;
        }
        let target = target.trim_start_matches('/').replace('\\', "/");
        let note_dir = self.active.as_ref().and_then(|path| path.parent().map(Path::to_path_buf)).unwrap_or_else(|| root.clone());
        [note_dir.join(&target), root.join(&target), root.join("_attachments").join(&target), note_dir.join("_attachments").join(&target)]
            .into_iter()
            .find(|candidate| candidate.is_file())
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

    /// Rebuilds the filtered tree when the filter or the tree changed. The sidebar calls this
    /// before drawing; [`Self::filtered_tree`] then hands back the cached slice for free.
    pub fn rebuild_filtered_tree(&mut self) {
        let filter = self.filter.trim().to_string();
        let stale =
            self.filtered_cache.as_ref().is_none_or(|(generation, cached_filter, _)| *generation != self.tree_generation || *cached_filter != filter);
        if !stale {
            return;
        }
        let nodes = if filter.is_empty() { self.tree.clone() } else { filter_tree(&self.tree, &self.index, &filter) };
        self.filtered_cache = Some((self.tree_generation, filter, nodes));
    }

    /// The tree as the sidebar shows it: the cache [`Self::rebuild_filtered_tree`] filled.
    pub fn filtered_tree(&self) -> &[vault::Node] {
        self.filtered_cache.as_ref().map(|(_, _, nodes)| nodes.as_slice()).unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A vault of this test on disk, removed when the guard drops.
    struct Scratch(PathBuf);

    impl Scratch {
        /// Writes `(relative path, text)` as the vault's notes.
        fn new(tag: &str, notes: &[(&str, &str)]) -> Self {
            let root = std::env::temp_dir().join(format!("localobsidian-flow-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("scratch dir");
            for (relative, text) in notes {
                let path = root.join(relative);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("parent");
                }
                std::fs::write(path, text).expect("write note");
            }
            Self(root)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn opened(scratch: &Scratch) -> VaultState {
        let mut state = VaultState::default();
        state.open(scratch.0.clone()).expect("open the vault");
        state
    }

    #[test]
    fn opening_a_vault_reads_the_tree_skips_dotfiles_and_builds_the_index() {
        let scratch = Scratch::new(
            "tree",
            &[
                ("Sepse.md", "#cardio e [[Choque]]"),
                ("Pasta/Choque.md", "---\ntags: [uti]\naliases: [choque septico]\n---\ncorpo"),
                (".oculto/Escondida.md", "#nada"),
                (".rascunho.md", "#nada"),
                ("Pasta/imagem.png", "binario"),
            ],
        );
        let state = opened(&scratch);
        assert_eq!(state.files.len(), 2, "{:?}", state.files.iter().map(|f| &f.name).collect::<Vec<_>>());
        assert_eq!(state.index.tag_count("cardio"), 1);
        assert_eq!(state.index.tag_count("uti"), 1);
        assert_eq!(state.index.aliases.get("choque septico"), Some(&scratch.0.join("Pasta/Choque.md")));
    }

    #[test]
    fn the_sidebar_lists_notes_before_folders() {
        let scratch = Scratch::new("order", &[("Zebra.md", ""), ("Alfa/Nota.md", "")]);
        let state = opened(&scratch);
        let names: Vec<&str> = state.tree.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(names, ["Zebra", "Alfa"]);
    }

    #[test]
    fn a_wikilink_resolves_to_the_note_it_names() {
        let scratch = Scratch::new("resolve", &[("Sepse.md", "[[Choque]]"), ("Pasta/Choque.md", "x")]);
        let state = opened(&scratch);
        assert_eq!(state.resolve("Choque").map(|file| file.relative_path.as_str()), Some("Pasta/Choque.md"));
        assert!(state.resolve("Nada").is_none());
    }

    #[test]
    fn an_alias_resolves_when_no_file_has_that_name() {
        let scratch = Scratch::new("alias", &[("Choque.md", "---\naliases: [choque septico]\n---\n")]);
        let state = opened(&scratch);
        assert_eq!(state.resolve("Choque Septico").map(|file| file.name.as_str()), Some("Choque"));
    }

    #[test]
    fn editing_then_saving_writes_the_buffer_and_clears_the_dirty_flag() {
        let scratch = Scratch::new("save", &[("Nota.md", "antes")]);
        let mut state = opened(&scratch);
        state.open_note(&scratch.0.join("Nota.md"));
        state.buffer = "depois".to_string();
        state.touch();
        assert!(state.dirty);
        state.save().expect("save");
        assert!(!state.dirty);
        assert_eq!(std::fs::read_to_string(scratch.0.join("Nota.md")).expect("read"), "depois");
        assert!(!vault::fs::temp_path_for(&scratch.0.join("Nota.md")).exists());
    }

    #[test]
    fn leaving_a_dirty_note_saves_it_first() {
        let scratch = Scratch::new("switch", &[("A.md", "a"), ("B.md", "b")]);
        let mut state = opened(&scratch);
        state.open_note(&scratch.0.join("A.md"));
        state.buffer = "editado".to_string();
        state.touch();
        state.open_note(&scratch.0.join("B.md"));
        assert_eq!(std::fs::read_to_string(scratch.0.join("A.md")).expect("read"), "editado");
    }

    #[test]
    fn a_new_note_starts_with_its_title_and_opens() {
        let scratch = Scratch::new("create", &[]);
        let mut state = opened(&scratch);
        let path = state.create_note("Receita").expect("create");
        assert_eq!(state.active.as_deref(), Some(path.as_path()));
        assert_eq!(state.buffer, "# Receita\n\n");
    }

    #[test]
    fn renaming_rewrites_the_links_that_pointed_at_the_note() {
        let scratch =
            Scratch::new("rename", &[("Nota.md", "# Nota"), ("Outra.md", "veja [[Nota]] e [[Nota#Seção|a seção]]"), ("Terceira.md", "[[Outra]]")]);
        let mut state = opened(&scratch);
        state.open_note(&scratch.0.join("Nota.md"));
        let count = state.rename_active("Nova").expect("rename");
        assert_eq!(count, 2, "two links changed");
        let other = std::fs::read_to_string(scratch.0.join("Outra.md")).expect("read");
        assert_eq!(other, "veja [[Nova]] e [[Nova#Seção|a seção]]");
        assert!(!scratch.0.join("Nota.md").exists());
        assert!(scratch.0.join("Nova.md").exists());
        assert_eq!(state.active, Some(scratch.0.join("Nova.md")));
    }

    #[test]
    fn the_filter_narrows_by_tag_and_alias_and_keeps_the_folders() {
        let scratch =
            Scratch::new("filter", &[("Pasta/Cardio.md", "#sistema/cardio"), ("Pasta/Outra.md", "#uti"), ("Raiz.md", "---\naliases: [SCA]\n---\n")]);
        let mut state = opened(&scratch);
        state.filter = "#cardio".to_string();
        state.rebuild_filtered_tree();
        let kept = state.filtered_tree();
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(kept[0].children.len(), 1);
        state.filter = "sca".to_string();
        state.rebuild_filtered_tree();
        assert_eq!(state.filtered_count(), 1);
    }

    #[test]
    fn the_preview_expands_an_embed_and_links_its_target() {
        let scratch = Scratch::new("preview", &[("Alvo.md", "# Um\n\ntexto alvo"), ("Nota.md", "antes ![[Alvo#Um]] depois [[Fim]]")]);
        let mut state = opened(&scratch);
        state.open_note(&scratch.0.join("Nota.md"));
        state.refresh_preview();
        let blocks = state.preview();

        let mut text = String::new();
        let mut links = Vec::new();
        fn walk(blocks: &[localobsidian_md::preview::Block], text: &mut String, links: &mut Vec<String>) {
            use localobsidian_md::preview::{Block, Span};
            for block in blocks {
                match block {
                    Block::Heading { spans, .. } | Block::Paragraph(spans) => {
                        for span in spans {
                            text.push_str(&span.plain());
                            if let Span::Link { destination, .. } = span
                                && let Some(target) = localobsidian_md::links::from_url(destination)
                            {
                                links.push(target);
                            }
                        }
                        text.push('\n');
                    }
                    Block::Quote(inner) => walk(inner, text, links),
                    Block::List { items, .. } => {
                        for item in items {
                            for span in item {
                                text.push_str(&span.plain());
                            }
                            text.push('\n');
                        }
                    }
                    Block::Code { text: code, .. } => text.push_str(code),
                    // The callout/table/image blocks keep their own text; the walk only needs the
                    // headings, paragraphs, quotes and lists this test puts in the note.
                    Block::Callout { title, body, .. } => {
                        text.push_str(title);
                        walk(body, text, links);
                    }
                    Block::Table { headers, rows, .. } => {
                        for cell in headers.iter().chain(rows.iter().flatten()) {
                            for span in cell {
                                text.push_str(&span.plain());
                            }
                        }
                    }
                    Block::Image { destination, .. } => text.push_str(destination),
                    Block::Rule => {}
                }
            }
        }
        walk(blocks, &mut text, &mut links);
        assert!(text.contains("texto alvo"), "{text}");
        // The embed is a callout: its head is the source, its body is the embedded note.
        assert!(text.contains("Alvo › Um"), "{text}");
        assert_eq!(links, ["Fim"]);
    }
}
