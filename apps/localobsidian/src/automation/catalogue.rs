//! The commands Local Obsidian offers an agent.
//!
//! The list is the vault app's own: the palette's commands (`src/app/commands.rs`) plus the read and
//! write side of the same document — `note.open`, `note.read`, `note.write`, `vault.inspect`,
//! `vault.search` and `vault.diagnostics` (what the editor, the sidebar, the search panel and the
//! diagnostics window show). Nothing here is a new capability; commands that need the window say
//! `appOnly` and the headless session refuses them, pointing at `--control`/`--connect`.

use serde_json::{Map, Value, json};

/// One command of the app.
pub struct Spec {
    /// The id the palette and the control channel use.
    pub id: &'static str,
    /// The English label the palette shows (the i18n key).
    pub label: &'static str,
    /// The menu it is listed under.
    pub menu: &'static str,
    /// The window is needed: a picker, the viewport, the review panel, the AI chat, the updater.
    pub app_only: bool,
}

/// Every command, in the palette's order.
pub const SPECS: &[Spec] = &[
    Spec { id: "file.open", label: "Open a vault…", menu: "File", app_only: false },
    Spec { id: "file.new", label: "New note…", menu: "File", app_only: false },
    Spec { id: "file.import_document", label: "Import a Word or OpenDocument file…", menu: "File", app_only: true },
    Spec { id: "file.import_anki", label: "Import Anki cards (text)…", menu: "File", app_only: true },
    Spec { id: "file.rename", label: "Rename note…", menu: "File", app_only: false },
    Spec { id: "file.save", label: "Save", menu: "File", app_only: false },
    Spec { id: "file.export_html", label: "Export the note as HTML…", menu: "File", app_only: false },
    Spec { id: "file.export_pdf", label: "Export the note as PDF…", menu: "File", app_only: false },
    Spec { id: "file.settings", label: "Settings…", menu: "File", app_only: true },
    Spec { id: "file.exit", label: "Exit", menu: "File", app_only: true },
    Spec { id: "note.open", label: "Open the note", menu: "File", app_only: false },
    Spec { id: "note.read", label: "Read the note", menu: "File", app_only: false },
    Spec { id: "note.write", label: "Write the note", menu: "File", app_only: false },
    Spec { id: "vault.inspect", label: "Inspect the vault", menu: "File", app_only: false },
    Spec { id: "vault.search", label: "Search the vault", menu: "File", app_only: false },
    Spec { id: "vault.diagnostics", label: "Vault diagnostics", menu: "File", app_only: false },
    Spec { id: "view.sidebar", label: "Show the sidebar", menu: "View", app_only: true },
    Spec { id: "view.mode.edit", label: "Note view: Edit", menu: "View", app_only: true },
    Spec { id: "view.mode.read", label: "Note view: Read", menu: "View", app_only: true },
    Spec { id: "view.mode.split", label: "Note view: Split", menu: "View", app_only: true },
    Spec { id: "view.theme.system", label: "Theme: System", menu: "View", app_only: true },
    Spec { id: "view.theme.light", label: "Theme: Light", menu: "View", app_only: true },
    Spec { id: "view.theme.dark", label: "Theme: Dark", menu: "View", app_only: true },
    Spec { id: "view.zoom_in", label: "Zoom in", menu: "View", app_only: true },
    Spec { id: "view.zoom_out", label: "Zoom out", menu: "View", app_only: true },
    Spec { id: "view.zoom_reset", label: "Reset the zoom", menu: "View", app_only: true },
    Spec { id: "view.sort.name", label: "Sort notes: Name", menu: "View", app_only: true },
    Spec { id: "view.sort.name_desc", label: "Sort notes: Name (Z to A)", menu: "View", app_only: true },
    Spec { id: "view.sort.modified", label: "Sort notes: Last modified", menu: "View", app_only: true },
    Spec { id: "view.search", label: "Search the vault", menu: "View", app_only: true },
    Spec { id: "view.outline", label: "Show the outline", menu: "View", app_only: true },
    Spec { id: "view.connections", label: "Show the connections", menu: "View", app_only: true },
    Spec { id: "view.diagnostics", label: "Vault diagnostics", menu: "View", app_only: true },
    Spec { id: "view.random", label: "Random note", menu: "View", app_only: true },
    Spec { id: "view.calendar", label: "Show the calendar", menu: "View", app_only: true },
    Spec { id: "view.refresh", label: "Refresh the vault", menu: "View", app_only: true },
    Spec { id: "review.vault", label: "Review the whole vault", menu: "Review", app_only: true },
    Spec { id: "review.note", label: "Review this note", menu: "Review", app_only: true },
    Spec { id: "review.tag", label: "Review #tag", menu: "Review", app_only: true },
    Spec { id: "review.stats", label: "Review statistics", menu: "Review", app_only: true },
    Spec { id: "review.close", label: "Close the review panel", menu: "Review", app_only: true },
    Spec { id: "ai.chat", label: "Chat about the note", menu: "AI", app_only: true },
    Spec { id: "ai.connection", label: "Connection…", menu: "AI", app_only: true },
    Spec { id: "ai.flashcards", label: "Create flashcards from this note", menu: "AI", app_only: true },
    Spec { id: "help.keys", label: "Keyboard shortcuts and help", menu: "Help", app_only: true },
    Spec { id: "help.updates", label: "Check for Updates…", menu: "Help", app_only: true },
    Spec { id: "help.about", label: "About Local Obsidian", menu: "Help", app_only: true },
];

/// The spec of `id`, if the app has that command.
pub fn spec(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.id == id)
}

/// Does the app have the command `id`?
pub fn has(id: &str) -> bool {
    spec(id).is_some()
}

/// The parameters of a command as a JSON Schema.
pub fn params(id: &str) -> Value {
    let text = |what: &str| json!({"type": "string", "description": what});
    let only = |props: Value| json!({"type": "object", "properties": props, "additionalProperties": false});
    let required = |mut schema: Value, keys: &[&str]| {
        schema["required"] = json!(keys);
        schema
    };
    let note = text("The note: a vault-relative path (`Pasta/Nota.md`), a note name, or an absolute path");
    match id {
        "file.open" => required(only(json!({"path": text("The vault folder to open")})), &["path"]),
        "file.new" => required(
            only(json!({
                "name": text("Name of the new note (without `.md`; the app adds it)"),
                "text": text("The note's whole text (default: the app's new-note text)"),
                "folder": text("Vault-relative folder for the note (default: the vault root)"),
            })),
            &["name"],
        ),
        "file.rename" => required(only(json!({ "path": note, "name": text("The new note name (without `.md`)") })), &["name"]),
        "file.export_html" | "file.export_pdf" => {
            required(only(json!({"path": text("File to write the export to (the format is the command's)"), "note": note})), &["path"])
        }
        "note.open" => required(only(json!({"path": note})), &["path"]),
        "note.read" => only(json!({"path": note})),
        "note.write" => {
            required(only(json!({"text": text("The note's whole text; it is written atomically, as the editor's save"), "path": note})), &["text"])
        }
        "vault.inspect" => only(json!({"limit": {"type": "integer", "minimum": 0, "description": "Most notes to list (default 500; 0 lists all)"}})),
        "vault.search" => required(
            only(json!({
                "query": text("The search box's text (words, `tag:x`, …)"),
                "regex": {"type": "boolean", "description": "Read the free text as a regular expression (the panel's `.*` switch)"},
                "limit": {"type": "integer", "minimum": 0, "description": "Most notes to return (default 50; 0 returns all)"},
            })),
            &["query"],
        ),
        _ => only(json!({})),
    }
}

/// The catalogue as `command_list` and `commands` return it, in palette order.
pub fn list() -> Vec<Value> {
    SPECS.iter().map(entry).collect()
}

fn entry(s: &'static Spec) -> Value {
    json!({"id": s.id, "label": s.label, "menu": s.menu, "appOnly": s.app_only, "params": params(s.id)})
}

/// The catalogue kept by `filter` (case-insensitive id, label or menu substring), and only the
/// ids and labels when `brief`. An empty `filter` keeps everything.
pub fn search(filter: Option<&str>, brief: bool) -> Vec<Value> {
    let needle = filter.map(str::to_lowercase).filter(|n| !n.is_empty());
    list()
        .into_iter()
        .filter(|c| match &needle {
            None => true,
            Some(n) => ["id", "label", "menu"].iter().any(|k| c.get(*k).and_then(Value::as_str).is_some_and(|v| v.to_lowercase().contains(n))),
        })
        .map(|c| {
            if brief {
                match c {
                    Value::Object(mut o) => {
                        o.retain(|k, _| k == "id" || k == "label");
                        Value::Object(o)
                    }
                    other => other,
                }
            } else {
                c
            }
        })
        .collect()
}

/// The parameters of one command as an object, refusing a key the command does not read.
pub fn arguments(id: &str, params: &Value) -> Result<Map<String, Value>, String> {
    let map = match params {
        Value::Null => Map::new(),
        Value::Object(m) => m.clone(),
        _ => return Err(format!("`{id}`: params must be a JSON object")),
    };
    let schema = crate::automation::catalogue::params(id);
    let allowed: Vec<&str> = schema
        .as_object()
        .and_then(|o| o.get("properties"))
        .and_then(Value::as_object)
        .map(|p| p.keys().map(String::as_str).collect())
        .unwrap_or_default();
    if let Some(key) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
        let expected = if allowed.is_empty() { "it takes no parameters".to_string() } else { format!("its parameters are {}", allowed.join(", ")) };
        return Err(format!("`{id}`: unknown parameter `{key}`: {expected}"));
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_palette_command_is_listed_once() {
        let mut ids: Vec<&str> = SPECS.iter().map(|s| s.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "a command id is listed twice");
        assert!(has("file.open") && has("note.write") && !has("note.delete"));
        assert!(spec("ai.chat").is_some_and(|s| s.app_only) && !spec("note.read").is_some_and(|s| s.app_only));
    }

    #[test]
    fn search_filters_by_id_label_or_menu_and_brief_keeps_the_names() {
        assert_eq!(search(None, false).len(), SPECS.len());
        assert_eq!(search(Some("THEME"), false).len(), 3);
        let review = search(Some("Review"), false);
        assert!(review.iter().all(|c| c["menu"] == "Review"));
        assert_eq!(
            search(Some("diagnostics"), true),
            vec![json!({"id": "vault.diagnostics", "label": "Vault diagnostics"}), json!({"id": "view.diagnostics", "label": "Vault diagnostics"}),]
        );
        assert!(search(Some("zzz"), false).is_empty());
        assert_eq!(search(Some(""), false).len(), SPECS.len(), "an empty filter keeps everything");
    }

    #[test]
    fn parameters_are_checked_against_the_schema() {
        assert_eq!(arguments("file.open", &json!({"path": "C:/notas"})).expect("a known key").len(), 1);
        let e = arguments("file.open", &json!({"folder": "C:/notas"})).expect_err("a typo is refused");
        assert!(e.contains("unknown parameter `folder`") && e.contains("path"), "{e}");
        let e = arguments("review.close", &json!({"id": 1})).expect_err("it takes none");
        assert!(e.contains("no parameters"), "{e}");
        assert!(arguments("review.close", &Value::Null).is_ok());
        assert!(arguments("review.close", &json!([1])).is_err());
    }
}
