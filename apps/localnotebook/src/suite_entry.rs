//! The app's own entry in the suite catalog, as `craft-update` knows apps.
//!
//! Every app of the suite that updates itself takes its entry from the catalog at
//! `craft_update::suite`, whose single source is `Anon5T4R/local-libs`
//! (`crates/update/src/suite.rs`); it reaches this repository through
//! `scripts/sync.ps1 -Mode push` as a mirror under `libs/craft-libs/crates/update`. The entry
//! `LOCALNOTEBOOK` holds this app's id, display name, tagline and repository, so the About window,
//! the update check and the installers all read the same values.

use craft_update::App;

/// This app, as its releases and installers know it.
pub const APP: App = craft_update::suite::LOCALNOTEBOOK;
