//! Watching the vault folder so a note changed elsewhere shows up here.
//!
//! The OS tells us a file changed; deciding what that *means* is the interesting part, and it is
//! what this module does not do - it only collects and debounces, and the window decides:
//!
//! - **Our own writes are not news.** Autosave stages a `.{name}.saving` beside the note and renames
//!   it; a save would come back as two events and reload the vault under the user's caret. The temp
//!   names are dropped here, and the note that was just written is filtered by the caller.
//! - **The state folder is not a note.** `.openobsidian/srs.json` changes on every review; a review
//!   session must not reload the vault.
//! - **A burst is one change.** Editors and sync clients write in flurries; the events are collected
//!   and only a quiet moment counts as a change ([`Watcher::settled`]).
//!
//! Behind it is the `notify` crate (pure Rust bindings to the OS API: ReadDirectoryChangesW on
//! Windows, inotify on Linux, kqueue on macOS). It has no C code of its own.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher as _};

/// How long the folder has to stay quiet before a change is acted on.
pub const SETTLE: Duration = Duration::from_millis(300);

/// The vault's folder as the operating system sees it changing.
pub struct Watcher {
    /// Kept alive: dropping it stops the watch.
    _watcher: RecommendedWatcher,
    events: Receiver<notify::Result<Event>>,
    /// The paths seen since the last time the caller took them.
    changed: Vec<PathBuf>,
}

impl Watcher {
    /// Starts watching `root` (recursively). `None` when the platform refuses - a vault that cannot
    /// be watched still works, it just needs the Refresh item in the menu.
    pub fn start(root: &Path) -> Option<Self> {
        let (sender, events) = channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            // A closed receiver means the window is gone; dropping the event is fine.
            let _ = sender.send(event);
        })
        .map_err(|error| log::warn!("watch: cannot start: {error}"))
        .ok()?;
        watcher.watch(root, RecursiveMode::Recursive).map_err(|error| log::warn!("watch: cannot watch {}: {error}", root.display())).ok()?;
        Some(Self { _watcher: watcher, events, changed: Vec::new() })
    }

    /// Collects what the OS reported since the last call. Returns `true` when something worth
    /// reloading happened (the caller drains [`Self::changed`] after `SETTLE`).
    pub fn poll(&mut self) -> bool {
        let mut saw_something = false;
        while let Ok(event) = self.events.try_recv() {
            let Ok(event) = event else { continue };
            for path in event.paths {
                if is_noise(&path) {
                    continue;
                }
                saw_something = true;
                if !self.changed.contains(&path) {
                    self.changed.push(path);
                }
            }
        }
        saw_something
    }

    /// The paths collected so far; the caller clears them by taking them.
    pub fn take(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.changed)
    }

    /// Whether the folder has been quiet long enough to act on what was collected.
    pub fn settled(since: Option<Instant>) -> bool {
        since.is_some_and(|at| at.elapsed() >= SETTLE)
    }
}

/// Files that never mean "a note changed": the atomic write's temp file, the state folder, and
/// anything hidden (which the tree does not show either).
fn is_noise(path: &Path) -> bool {
    let name = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
    if name.starts_with('.') || name.ends_with(".saving") {
        return true;
    }
    path.components().any(|component| {
        let part = component.as_os_str().to_string_lossy();
        part == ".openobsidian" || part == ".git"
    })
}

/// Whether a changed path is a note the tree would list.
pub fn is_note_path(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("md")) && !is_noise(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_atomic_writes_temp_file_is_noise() {
        assert!(is_noise(Path::new("/v/Nota/.Nota.md.saving")));
        assert!(is_noise(Path::new("/v/.openobsidian/srs.json")));
        assert!(is_noise(Path::new("/v/.rascunho.md")));
        assert!(!is_noise(Path::new("/v/Nota.md")));
    }

    #[test]
    fn only_markdown_notes_are_notes() {
        assert!(is_note_path(Path::new("/v/Nota.md")));
        assert!(is_note_path(Path::new("/v/Nota.MD")));
        assert!(!is_note_path(Path::new("/v/foto.png")));
        assert!(!is_note_path(Path::new("/v/.oculta.md")));
    }

    #[test]
    fn a_quiet_folder_settles() {
        assert!(Watcher::settled(Some(Instant::now() - SETTLE)));
        assert!(!Watcher::settled(Some(Instant::now())));
        assert!(!Watcher::settled(None));
    }
}
