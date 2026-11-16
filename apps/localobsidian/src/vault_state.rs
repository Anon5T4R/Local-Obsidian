//! The open vault, as the app holds it: the tree, the note list, the index, the note being edited
//! and everything that changes them. No egui here - the views read this state and hand back what
//! the user asked for.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use localobsidian_md::{embeds, links, outline, preview};
use localobsidian_review as review;
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

    /// Whether the reading view is on screen (alone or beside the editor).
    pub fn shows_reading(self) -> bool {
        matches!(self, ViewMode::Read | ViewMode::Split)
    }
}

/// The vault and the note in front of the user.
///
/// Hand-written `Default` rather than derived, because the divider starts in the middle (a derived
/// one would start a drag away from zero).
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
    /// The tag chip that is filtering the tree, if any.
    pub tag_filter: Option<String>,
    /// The search panel.
    pub search: SearchState,
    /// Whether the outline panel is on screen.
    pub outline_open: bool,
    /// Whether the sidebar's tag section is unfolded. Collapsed by default: the tree is the way one
    /// navigates, and a vault can carry hundreds of tags.
    pub tags_open: bool,
    /// Where the divider between the source and the reading view sits, as a share of the width.
    pub split: f32,
    /// How much the reading view is scaled: 0.8 to 1.6, 1.0 by default.
    pub zoom: f32,
    /// The open note's frontmatter, as `(key, value)` in file order, refreshed with the preview.
    properties: Vec<(String, String)>,
    /// The section a link asked for: the reading view scrolls to it, then clears it.
    pub pending_section: Option<String>,
    /// The line a search result asked for: the editor puts the caret there, then clears it.
    pub pending_line: Option<usize>,
    /// The `[[wikilink` being typed, when the popup is open.
    pub completion: Option<crate::completions::Completion>,
    /// The notes already visited, for the two arrows of the note header.
    pub history: crate::history::History,
    /// The schedule in `.openobsidian/srs.json`, as the phone reads it.
    pub srs: review::SrsFile,
    /// The schedule changed and has not reached the disk yet.
    pub srs_dirty: bool,
    /// The cached reading view of [`Self::revision`].
    preview_cache: Option<(u64, Vec<preview::Block>)>,
    /// The cached outline of [`Self::revision`].
    outline_cache: Option<(u64, Vec<outline::Heading>)>,
    /// The filtered tree, and the inputs it was built from (the filter and the tree's generation).
    filtered_cache: Option<(u64, String, Option<String>, Vec<vault::Node>)>,
    /// Bumped whenever the tree changes, so the filtered view knows when to rebuild.
    tree_generation: u64,
    contents: BTreeMap<PathBuf, String>,
    last_edit: Option<Instant>,
}

impl Default for VaultState {
    fn default() -> Self {
        Self {
            root: None,
            tree: Vec::new(),
            files: Vec::new(),
            index: vault::Index::default(),
            active: None,
            buffer: String::new(),
            dirty: false,
            filter: String::new(),
            sort: vault::Sort::default(),
            view: ViewMode::default(),
            message: None,
            revision: 0,
            tag_filter: None,
            search: SearchState::default(),
            outline_open: false,
            tags_open: false,
            split: 0.5,
            zoom: 1.0,
            properties: Vec::new(),
            pending_section: None,
            pending_line: None,
            completion: None,
            history: crate::history::History::default(),
            srs: review::SrsFile::default(),
            srs_dirty: false,
            preview_cache: None,
            outline_cache: None,
            filtered_cache: None,
            tree_generation: 0,
            contents: BTreeMap::new(),
            last_edit: None,
        }
    }
}

/// The search panel's state: the box, its switches, and the results of the last run.
#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    /// Match the free text as a regular expression.
    pub regex: bool,
    /// The pattern did not compile: shown under the box instead of searching.
    pub invalid: bool,
    pub results: Vec<SearchHit>,
    /// Notes whose every matching line is shown (the panel shows a few first).
    pub expanded: std::collections::HashSet<PathBuf>,
    /// What the results belong to (the box and the `.*` switch), so a frame that changes nothing
    /// does not walk the vault again.
    results_for: Option<(String, bool)>,
    /// Input waiting for the typing to settle, and since when.
    pending: Option<((String, bool), Instant)>,
}

impl SearchState {
    /// Forgets the results, so the next [`VaultState::refresh_search`] walks the vault again (used
    /// when the box or the `.*` switch changes, or when the panel is closed).
    pub fn invalidate(&mut self) {
        self.clear();
    }

    fn clear(&mut self) {
        self.results_for = None;
        self.pending = None;
        self.results.clear();
    }
}

/// One note in the search results.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub path: PathBuf,
    pub name: String,
    pub relative_path: String,
    pub matches: Vec<vault::search::LineMatch>,
    pub score: f32,
}

impl VaultState {
    /// Opens `root` as a vault: scans the tree, reads every note and builds the index.
    pub fn open(&mut self, root: PathBuf) -> io::Result<()> {
        self.root = Some(root);
        self.active = None;
        self.buffer.clear();
        self.dirty = false;
        self.filter.clear();
        // Another vault: the arrows have nowhere to go.
        self.history.clear();
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
        // The notes changed under the results: the next frame that draws the panel searches again.
        self.search.invalidate();
        // The schedule is shared with the phone: read it again and reconcile it with the notes.
        self.load_and_sync_srs();

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
        let from = self.active.clone();
        if self.load_note(path) {
            self.history.visit(from.as_deref(), path);
        }
    }

    /// Reads `path` into the buffer without touching the history. Returns whether it opened.
    fn load_note(&mut self, path: &Path) -> bool {
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
                self.outline_cache = None;
                self.properties.clear();
                self.completion = None;
                true
            }
            Err(error) => {
                self.message = Some(format!("{}: {error}", path.display()));
                false
            }
        }
    }

    /// The note the back arrow goes to, opened. Returns false at the first note of the session.
    pub fn go_back(&mut self) -> bool {
        let Some(current) = self.active.clone() else { return false };
        let Some(target) = self.history.peek_back() else { return false };
        if !self.load_note(&target) {
            // The note is gone from the vault: the step is not taken, so the arrow does not lie
            // about where it goes (and the next one can still be reached).
            return false;
        }
        self.history.step_back(&current);
        true
    }

    /// The note the forward arrow returns to, opened.
    pub fn go_forward(&mut self) -> bool {
        let Some(current) = self.active.clone() else { return false };
        let Some(target) = self.history.peek_forward() else { return false };
        if !self.load_note(&target) {
            return false;
        }
        self.history.step_forward(&current);
        true
    }

    /// Marks the buffer as edited: dirty, a new revision for the reading view, autosave due.
    pub fn touch(&mut self) {
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
        self.last_edit = Some(Instant::now());
    }

    /// Replaces the whole buffer (an edit made from the reading view, which has no caret), and marks
    /// it edited. The preview is rebuilt on the next frame.
    pub fn replace_buffer(&mut self, text: String) {
        if self.active.is_none() || self.buffer == text {
            return;
        }
        self.buffer = text;
        self.preview_cache = None;
        self.outline_cache = None;
        self.touch();
    }

    /// Flips the `index`-th task checkbox of the open note (the reading view clicked it).
    pub fn toggle_task(&mut self, index: usize) -> bool {
        let Some(edit) = localobsidian_editor::toggle_task(&self.buffer, index) else { return false };
        self.replace_buffer(edit.text);
        true
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
        self.sync_note_srs(&path);
        Ok(())
    }

    /// Reconciles one note's cards with the schedule after it was written: a new card starts due
    /// today, and a card whose question disappeared leaves the schedule.
    fn sync_note_srs(&mut self, path: &Path) {
        let Some(relative) = self.files.iter().find(|note| note.path == path).map(|note| note.relative_path.clone()) else { return };
        let Some(content) = self.contents.get(path) else { return };
        let cards: Vec<(String, String)> = review::extract_cards(&relative, content).into_iter().map(|card| (card.id, card.q)).collect();
        let today = review::today();
        let (next, added, removed) = review::sync_file(&self.srs, &relative, &cards, &today);
        if added > 0 || removed > 0 || next != self.srs {
            self.srs = next;
            self.srs_dirty = true;
            self.save_srs();
        }
    }

    /// Re-reads the open note from disk, keeping the caret's text only when it is dirty.
    pub fn reload_open_note(&mut self) {
        let Some(path) = self.active.clone() else { return };
        if self.dirty {
            return;
        }
        match vault::read_note(&path) {
            Ok(text) => {
                self.buffer = text;
                self.revision = self.revision.wrapping_add(1);
                self.preview_cache = None;
                self.outline_cache = None;
                if let Some(entry) = self.contents.get_mut(&path) {
                    *entry = self.buffer.clone();
                }
            }
            Err(error) => self.message = Some(format!("{}: {error}", path.display())),
        }
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
        self.active = Some(renamed.clone());
        // The notes visited before the rename point at the file that moved, not at a note that is
        // not there any more.
        self.history.retarget(&path, &renamed);
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
            // The note's own fields, for the card the reading view draws at the top.
            self.properties = localobsidian_md::frontmatter::parse(&self.buffer).map(|frontmatter| frontmatter.pairs()).unwrap_or_default();
        }
    }

    /// The reading view as last built by [`Self::refresh_preview`].
    pub fn preview(&self) -> &[preview::Block] {
        self.preview_cache.as_ref().map(|(_, blocks)| blocks.as_slice()).unwrap_or_default()
    }

    /// The open note's frontmatter, in file order (empty when it has none).
    pub fn frontmatter(&self) -> &[(String, String)] {
        &self.properties
    }

    /// Rebuilds the filtered tree when the filter, the tag chip or the tree changed. The sidebar
    /// calls this before drawing; [`Self::filtered_tree`] then hands back the cached slice for free.
    pub fn rebuild_filtered_tree(&mut self) {
        let filter = self.filter.trim().to_string();
        let tag = self.tag_filter.clone();
        let stale = self.filtered_cache.as_ref().is_none_or(|(generation, cached_filter, cached_tag, _)| {
            *generation != self.tree_generation || *cached_filter != filter || *cached_tag != tag
        });
        if !stale {
            return;
        }
        let nodes =
            if filter.is_empty() && tag.is_none() { self.tree.clone() } else { filter_tree(&self.tree, &self.index, &filter, tag.as_deref()) };
        self.filtered_cache = Some((self.tree_generation, filter, tag, nodes));
    }

    /// The tree as the sidebar shows it: the cache [`Self::rebuild_filtered_tree`] filled.
    pub fn filtered_tree(&self) -> &[vault::Node] {
        self.filtered_cache.as_ref().map(|(_, _, _, nodes)| nodes.as_slice()).unwrap_or_default()
    }

    /// How many notes the current filter keeps.
    pub fn filtered_count(&self) -> usize {
        self.filtered_tree().iter().map(count_notes).sum()
    }

    /// Every tag of the vault with how many notes carry it, most used first.
    pub fn tag_counts(&self) -> Vec<(String, usize)> {
        let mut tags: Vec<(String, usize)> = self.index.tags.iter().map(|(tag, notes)| (tag.clone(), notes.len())).collect();
        tags.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        tags
    }

    /// The notes that link the open note (a `[[Note#Section]]` counts as a mention of `Note`).
    pub fn backlinks_of_active(&self) -> Vec<String> {
        let Some(active) = &self.active else { return Vec::new() };
        let Some(name) = active.file_stem().and_then(|stem| stem.to_str()) else { return Vec::new() };
        self.backlinks_of(name)
    }

    /// The notes that link `name`.
    pub fn backlinks_of(&self, name: &str) -> Vec<String> {
        let wanted = name.to_lowercase();
        let mut sources: Vec<String> = Vec::new();
        for (target, notes) in &self.index.backlinks {
            // `[[Nota#Seção]]` and `[[Nota]]` are both mentions of the note.
            if localobsidian_md::links::parse_target(target).note.to_lowercase() != wanted {
                continue;
            }
            for note in notes {
                if !sources.iter().any(|known| known.eq_ignore_ascii_case(note)) {
                    sources.push(note.clone());
                }
            }
        }
        sources.sort_by_key(|note| note.to_lowercase());
        sources
    }

    /// The text of a note, as last read or written.
    pub fn contents_of(&self, path: &Path) -> Option<&String> {
        self.contents.get(path)
    }

    /// Resolves a wikilink as if it were written in `from` (the graph needs this per note).
    pub fn resolve_note_from(&self, target: &str, from: Option<&Path>) -> Option<&vault::NoteFile> {
        vault::resolve_note(&self.files, target, from, Some(&self.index.aliases))
    }

    /// Writes a note's whole text (a note created from a template).
    pub fn create_note_with(&mut self, name: &str, content: &str, folder: Option<&Path>) -> io::Result<PathBuf> {
        let root = self.root.clone().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no vault is open"))?;
        let folder =
            folder.map(Path::to_path_buf).or_else(|| self.active.as_ref().and_then(|path| path.parent().map(Path::to_path_buf))).unwrap_or(root);
        let path = vault::create_note(&folder, name)?;
        vault::write_atomic(&path, content)?;
        self.reload()?;
        self.open_note(&path);
        Ok(path)
    }

    /// Where the shared schedule lives: `.openobsidian/srs.json` in the vault.
    pub fn srs_path(&self) -> Option<PathBuf> {
        self.root.as_ref().map(|root| root.join(".openobsidian").join("srs.json"))
    }

    /// Every card the notes currently declare, keyed by the note's relative path.
    fn cards_in_vault(&self) -> BTreeMap<String, Vec<(String, String)>> {
        let mut found = BTreeMap::new();
        for note in &self.files {
            let Some(content) = self.contents.get(&note.path) else { continue };
            let cards = review::extract_cards(&note.relative_path, content);
            found.insert(note.relative_path.clone(), cards.into_iter().map(|card| (card.id, card.q)).collect());
        }
        found
    }

    /// Reads `.openobsidian/srs.json` and reconciles it with the notes, the same way both other
    /// apps do on open. A card keeps its schedule while its question does not change.
    fn load_and_sync_srs(&mut self) {
        let Some(path) = self.srs_path() else { return };
        let (file, dropped) = review::srs::load(&path);
        let today = review::today();
        let synced = review::sync_vault(&file, &self.cards_in_vault(), &today);
        self.srs_dirty = dropped > 0 || synced != file;
        self.srs = synced;
        if self.srs_dirty {
            self.save_srs();
        }
    }

    /// Writes the schedule. Returns whether it reached the disk.
    pub fn save_srs(&mut self) -> bool {
        let Some(path) = self.srs_path() else { return false };
        match review::srs::save(&path, &self.srs) {
            Ok(()) => {
                self.srs_dirty = false;
                true
            }
            Err(error) => {
                log::warn!("srs: cannot write {}: {error}", path.display());
                self.message = Some(format!("{}: {error}", crate::i18n::tr("could not write the review schedule")));
                false
            }
        }
    }

    /// The schedule's numbers, for the toolbar badge and the statistics panel.
    pub fn srs_stats(&self) -> review::SrsStats {
        review::stats(&self.srs, &review::today())
    }

    /// The statistics panel's report.
    pub fn srs_report(&self) -> review::SrsReport {
        review::report(&self.srs, &review::today())
    }

    /// Grades a card and writes the schedule at once, so a crash cannot lose a review session.
    pub fn grade_card(&mut self, id: &str, grade: review::Grade) -> bool {
        let today = review::today();
        let Some(card) = self.srs.cards.get(id).cloned() else { return false };
        self.srs.cards.insert(id.to_string(), review::grade(&card, grade, &today));
        self.srs_dirty = true;
        self.save_srs()
    }

    /// Parks a card the user cannot answer yet; `suspended: false` brings it back.
    pub fn suspend_card(&mut self, id: &str, suspended: bool) {
        if let Some(card) = self.srs.cards.get_mut(id) {
            card.suspended = suspended;
            self.srs_dirty = true;
            self.save_srs();
        }
    }

    /// The answer of a card, read from the note it lives in: an edited answer shows on the next
    /// review without any re-sync.
    pub fn card_answer(&self, id: &str) -> Option<String> {
        let card = self.srs.cards.get(id)?;
        let note = self.note_for_card(&card.file)?;
        let content = self.contents.get(&note.path)?;
        review::extract_cards(&note.relative_path, content).into_iter().find(|found| found.id == id).map(|found| found.a)
    }

    /// The note a card's `file` (relative path) points at. Cards scheduled before the relative-path
    /// fix carry an absolute path, so both shapes are accepted.
    pub fn note_for_card(&self, file: &str) -> Option<&vault::NoteFile> {
        let wanted = file.replace('\\', "/");
        self.files
            .iter()
            .find(|note| note.relative_path.replace('\\', "/") == wanted)
            .or_else(|| self.files.iter().find(|note| note.path.to_string_lossy().replace('\\', "/") == wanted))
            .or_else(|| self.files.iter().find(|note| note.path.to_string_lossy().replace('\\', "/").ends_with(&format!("/{wanted}"))))
    }

    /// The outline of the open note, rebuilt when its text changed.
    pub fn refresh_outline(&mut self) {
        let stale = self.outline_cache.as_ref().is_none_or(|(revision, _)| *revision != self.revision);
        if stale {
            self.outline_cache = Some((self.revision, outline::outline(&self.buffer)));
        }
    }

    /// The outline as last built by [`Self::refresh_outline`].
    pub fn outline(&self) -> &[outline::Heading] {
        self.outline_cache.as_ref().map(|(_, headings)| headings.as_slice()).unwrap_or_default()
    }

    /// Opens the note a search result points at, with the caret on `line` when the caller wants it.
    pub fn show_result(&mut self, path: &Path, line: Option<usize>) {
        self.open_note(path);
        self.pending_line = line;
        if line.is_some() {
            // A line only means something in the source.
            self.view = ViewMode::Edit;
        }
    }

    /// Re-runs the search when the box changed and the typing settled. The panel calls this every
    /// frame; it is a no-op once the results match the box.
    pub fn refresh_search(&mut self) {
        if !self.search.open {
            return;
        }
        let query = self.search.query.trim().to_string();
        self.search.invalid = false;
        if query.is_empty() {
            self.search.clear();
            return;
        }
        let key = (query, self.search.regex);
        // Nothing to do: the results already belong to what the box says.
        if self.search.results_for.as_ref() == Some(&key) {
            return;
        }

        // Wait for a pause in the typing: walking every note on each keystroke would make the box
        // feel heavy in a big vault.
        const SETTLE: Duration = Duration::from_millis(200);
        match &self.search.pending {
            Some((pending, since)) if *pending == key => {
                if since.elapsed() < SETTLE {
                    return;
                }
            }
            _ => {
                self.search.pending = Some((key, Instant::now()));
                return;
            }
        }
        self.search.pending = None;
        self.search.results_for = Some(key);
        let query = self.search.query.trim().to_string();
        self.run_search(&query);
    }

    /// Runs the search now, without waiting for the typing to settle (a test, or an explicit
    /// Enter in the box).
    pub fn run_search_now(&mut self) {
        let query = self.search.query.trim().to_string();
        self.search.invalid = false;
        self.search.pending = None;
        self.search.results_for = Some((query.clone(), self.search.regex));
        if query.is_empty() {
            self.search.results.clear();
            return;
        }
        self.run_search(&query);
    }

    fn run_search(&mut self, raw: &str) {
        let query = vault::search::parse_query(raw);
        if query.is_empty() {
            self.search.results.clear();
            return;
        }
        let pattern = self.search.regex.then(|| vault::search::compile_pattern(&query.needles().join(" ")));
        if self.search.regex && pattern.as_ref().is_none_or(Option::is_none) {
            self.search.invalid = true;
            self.search.results.clear();
            return;
        }
        let pattern = pattern.flatten();

        let mut hits: Vec<SearchHit> = Vec::new();
        for file in &self.files {
            let Some(content) = self.contents.get(&file.path) else { continue };
            let info = self.index.info_of(&file.path);
            let searchable =
                vault::search::Searchable { name: &file.name, relative_path: &file.relative_path, content, tags: &info.tags, aliases: &info.aliases };
            let Some(matches) = vault::search::search_with(&searchable, &query, pattern.as_ref()) else { continue };
            let score = vault::search::score(&searchable, &query, &matches);
            hits.push(SearchHit { path: file.path.clone(), name: file.name.clone(), relative_path: file.relative_path.clone(), matches, score });
        }
        // Most relevant first: alphabetical order would bury the note the query is about under
        // every note that merely mentions the word.
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        self.search.results = hits;
        self.search.expanded.clear();
    }
}

fn stem(path: &Path) -> String {
    path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default().to_string()
}

fn filter_tree(nodes: &[vault::Node], index: &vault::Index, query: &str, tag: Option<&str>) -> Vec<vault::Node> {
    let mut out = Vec::new();
    for node in nodes {
        match node.kind {
            vault::Kind::File => {
                let info = index.info_of(&node.path);
                if let Some(tag) = tag
                    && !info.tags.iter().any(|own| own == tag)
                {
                    continue;
                }
                let view = vault::Filterable { name: &node.name, tags: &info.tags, aliases: &info.aliases };
                if vault::matches_filter(&view, query) {
                    out.push(node.clone());
                }
            }
            // A document is not filtered by name or tag: the filter is about notes, and a
            // PDF that matched a query would look like a note it is not.
            vault::Kind::Binary => {}
            vault::Kind::Directory => {
                let mut kept = node.clone();
                kept.children = filter_tree(&node.children, index, query, tag);
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
        vault::Kind::Binary => 0,
        vault::Kind::Directory => node.children.iter().map(count_notes).sum(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gives the tests a vault without touching a disk (the graph's layout is pure).
    impl VaultState {
        pub fn replace_contents_for_tests(&mut self, contents: BTreeMap<PathBuf, String>) {
            self.contents = contents;
        }
    }

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
        let path = state.create_note_with("Receita", "# Receita\n\n", None).expect("create");
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
