//! The settings the app remembers between runs: the last vault, the sidebar order, whether the
//! reading view was open, and the interface language.
//!
//! One small JSON file in the platform's config folder (`%APPDATA%\LocalObsidian` on Windows,
//! `~/Library/Application Support/LocalObsidian` on macOS, `$XDG_CONFIG_HOME/localobsidian`
//! otherwise). A damaged file is not fatal: the defaults are used, and the next save replaces it.
//! `LOCALOBSIDIAN_NO_PREFS=1` turns the file off (tests, scripted runs).

use std::path::PathBuf;

use localobsidian_vault::Sort;

use crate::vault_state::ViewMode;

/// The divider between the source and the reading view can be dragged, but not past these: a pane
/// narrower than this is a pane one cannot read.
pub const SPLIT_MIN: f32 = 0.2;
pub const SPLIT_MAX: f32 = 0.8;

/// The reading view's scale: small enough to fit a wide table, large enough to read from far away.
pub const ZOOM_MIN: f32 = 0.8;
pub const ZOOM_MAX: f32 = 1.6;

/// What the app remembers. Every field has a default that is safe to start from.
#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    pub last_vault: Option<PathBuf>,
    pub sort: Sort,
    pub view: ViewMode,
    pub sidebar_open: bool,
    /// Whether the outline panel was on screen.
    pub outline_open: bool,
    /// Whether the tag section of the sidebar was unfolded. Collapsed by default: a vault of 148
    /// tags would otherwise bury the tree, which is the way one navigates.
    pub tags_open: bool,
    /// Where the divider between the source and the reading view sits, as a share of the width.
    pub split: f32,
    /// How much the reading view is scaled, 0.8 to 1.6.
    pub zoom: f32,
    /// [`crate::i18n::AUTO`] or a language code.
    pub language: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            last_vault: None,
            sort: Sort::Name,
            view: ViewMode::Edit,
            sidebar_open: true,
            outline_open: false,
            tags_open: false,
            split: 0.5,
            zoom: 1.0,
            language: crate::i18n::AUTO.to_string(),
        }
    }
}

impl Prefs {
    /// Reads the file, falling back to the defaults when it is missing or unusable.
    pub fn load() -> Self {
        let Some(path) = prefs_path() else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&path) else { return Self::default() };
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(value) => Self::from_json(&value),
            Err(error) => {
                log::warn!("prefs: {} is not valid JSON ({error}); using the defaults", path.display());
                Self::default()
            }
        }
    }

    /// Writes the file. A failure is logged, never fatal: settings are a convenience.
    pub fn save(&self) {
        let Some(path) = prefs_path() else { return };
        let text = self.to_json().to_string();
        if let Err(error) = localobsidian_vault::write_atomic(&path, &text) {
            log::warn!("prefs: cannot write {}: {error}", path.display());
        }
    }

    fn from_json(value: &serde_json::Value) -> Self {
        let mut prefs = Self::default();
        prefs.last_vault = value.get("last_vault").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(PathBuf::from);
        prefs.sort = match value.get("sort").and_then(|v| v.as_str()) {
            Some("name-desc") => Sort::NameDesc,
            Some("modified") => Sort::Modified,
            _ => Sort::Name,
        };
        // `preview_open` is the settings file of the first release: a reading view that was on is
        // the Read mode, and one that was off is the editor.
        prefs.view = match value.get("view").and_then(|v| v.as_str()) {
            Some(key) => ViewMode::from_key(key),
            None if value.get("preview_open").and_then(|v| v.as_bool()).unwrap_or(false) => ViewMode::Read,
            None => ViewMode::Edit,
        };
        prefs.sidebar_open = value.get("sidebar_open").and_then(|v| v.as_bool()).unwrap_or(prefs.sidebar_open);
        prefs.outline_open = value.get("outline_open").and_then(|v| v.as_bool()).unwrap_or(prefs.outline_open);
        prefs.tags_open = value.get("tags_open").and_then(|v| v.as_bool()).unwrap_or(prefs.tags_open);
        if let Some(split) = value.get("split").and_then(|v| v.as_f64()) {
            prefs.split = (split as f32).clamp(SPLIT_MIN, SPLIT_MAX);
        }
        if let Some(zoom) = value.get("zoom").and_then(|v| v.as_f64()) {
            prefs.zoom = (zoom as f32).clamp(ZOOM_MIN, ZOOM_MAX);
        }
        if let Some(language) = value.get("language").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            prefs.language = language.to_string();
        }
        prefs
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "last_vault": self.last_vault.as_ref().map(|path| path.to_string_lossy()),
            "sort": match self.sort {
                Sort::Name => "name",
                Sort::NameDesc => "name-desc",
                Sort::Modified => "modified",
            },
            "view": self.view.key(),
            "sidebar_open": self.sidebar_open,
            "outline_open": self.outline_open,
            "tags_open": self.tags_open,
            "split": self.split,
            "zoom": self.zoom,
            "language": self.language,
        })
    }
}

/// `<config>/LocalObsidian/prefs.json`, or `None` when the app must not touch settings.
fn prefs_path() -> Option<PathBuf> {
    if std::env::var_os("LOCALOBSIDIAN_NO_PREFS").is_some() {
        return None;
    }
    config_dir().map(|dir| dir.join("prefs.json"))
}

/// The app's config folder, by platform convention.
fn config_dir() -> Option<PathBuf> {
    let dir = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)?.join("LocalObsidian")
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/LocalObsidian")
    } else {
        match std::env::var_os("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
        }
        .join("localobsidian")
    };
    Some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_safe() {
        let prefs = Prefs::default();
        assert!(prefs.last_vault.is_none());
        assert_eq!(prefs.sort, Sort::Name);
        assert!(prefs.sidebar_open);
        assert_eq!(prefs.view, ViewMode::Edit);
    }

    #[test]
    fn a_written_file_reads_back_the_same() {
        let prefs = Prefs {
            last_vault: Some(PathBuf::from("C:/vault")),
            sort: Sort::Modified,
            view: ViewMode::Split,
            sidebar_open: false,
            outline_open: true,
            tags_open: true,
            split: 0.62,
            zoom: 1.2,
            language: "pt-br".to_string(),
        };
        let again = Prefs::from_json(&prefs.to_json());
        assert_eq!(again, prefs);
    }

    #[test]
    fn the_tags_start_collapsed_and_the_divider_in_the_middle() {
        let prefs = Prefs::default();
        assert!(!prefs.tags_open, "a vault of tags must not bury the tree");
        assert_eq!(prefs.split, 0.5);
        let again = Prefs::from_json(&serde_json::json!({}));
        assert!(!again.tags_open);
        assert_eq!(again.split, 0.5);
    }

    #[test]
    fn a_divider_off_the_rails_is_pulled_back() {
        // A hand-edited file (or a window that was very wide) cannot leave a pane of zero width.
        assert_eq!(Prefs::from_json(&serde_json::json!({ "split": 0.02 })).split, SPLIT_MIN);
        assert_eq!(Prefs::from_json(&serde_json::json!({ "split": 3.0 })).split, SPLIT_MAX);
    }

    #[test]
    fn the_first_releases_settings_file_still_reads() {
        let legacy = Prefs::from_json(&serde_json::json!({ "preview_open": true }));
        assert_eq!(legacy.view, ViewMode::Read);
        let legacy = Prefs::from_json(&serde_json::json!({ "preview_open": false }));
        assert_eq!(legacy.view, ViewMode::Edit);
    }

    #[test]
    fn missing_fields_keep_the_defaults() {
        let again = Prefs::from_json(&serde_json::json!({ "sort": "name-desc" }));
        assert_eq!(again.sort, Sort::NameDesc);
        assert_eq!(again.language, crate::i18n::AUTO);
        assert!(again.last_vault.is_none());
    }

    #[test]
    fn an_unknown_sort_falls_back_to_name() {
        assert_eq!(Prefs::from_json(&serde_json::json!({"sort": "aleatorio"})).sort, Sort::Name);
    }
}
