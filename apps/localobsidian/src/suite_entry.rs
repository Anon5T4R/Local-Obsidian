//! The app's own entry as `craft-update` knows apps.
//!
//! The suite's single source is `craft_update::suite` (in `Anon5T4R/local-libs`
//! `crates/update/src/suite.rs`), mirrored into this repository as
//! `libs/craft-libs/crates/update` by `scripts/sync.ps1 -Mode push`. The born app's entry
//! (`LOCALOBSIDIAN`) is added there; until the mirror arrives this file declares the same `App`
//! value by hand, and then [`APP`] points at the catalog:
//!
//! ```ignore
//! pub const APP: craft_update::App = craft_update::suite::LOCALOBSIDIAN;
//! ```

use craft_update::App;

/// This app, as its releases and installers know it.
pub const APP: App = App {
    publisher: "Local Obsidian contributors",
    ..App::new(
        // Machine name: repo, binary, package and asset stem (`localobsidian-0.1.0-windows-x64.msi`).
        "localobsidian",
        // Display name: window title, installer, menu entries.
        "Local Obsidian",
        // What it is for, in a few words (English); installers write it into the menu entry.
        "Notes, wikilinks and attachments in a plain folder",
        // GitHub `owner/repo`: releases are only ever downloaded from here.
        "Anon5T4R/Local-Obsidian",
    )
};
