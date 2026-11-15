//! The vault: a folder of Markdown notes, read as files.
//!
//! Nothing here knows about egui. The app asks this crate what the vault holds ([`scan`]), what a
//! note links to ([`index`]), which file a `[[link]]` means ([`resolve_note`]) and how to write a
//! note without ever leaving it half-written ([`write_atomic`]).
//!
//! Rules ported from OpenObsidian: `walkTree` (what the tree hides), `treeSort.ts` (files before
//! folders), `sidebarFilter.ts` (a filter matches names, tags and aliases, never note text),
//! `vaultStore.buildBacklinks` (the index) and `safe-write.ts` (temp file + rename).

pub mod filter;
pub mod fs;
pub mod index;
pub mod resolve;
pub mod tree;

pub use filter::{Filterable, matches_filter};
pub use fs::{create_note, read_note, rename_note, write_atomic};
pub use index::Index;
pub use resolve::resolve_note;
pub use tree::{Kind, Node, NoteFile, Sort, flatten, scan, sorted};

/// Where a note sits in the vault, for the app's title bar and tooltips.
pub fn relative_display(vault_root: &std::path::Path, note: &std::path::Path) -> String {
    note.strip_prefix(vault_root).unwrap_or(note).to_string_lossy().replace('\\', "/")
}
