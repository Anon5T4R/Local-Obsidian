//! Running Local Notebook's commands without a window: a vault open, its notes read and written
//! through the app's own state.
//!
//! The same functions serve the headless session ([`Session`], behind `localnotebook mcp`,
//! `run --script` and `commands`) and the running window's control channel (`--control PORT`): the
//! app passes its own [`VaultState`], so `mcp --connect` and the headless server work through the
//! same code the sidebar, the editor, the search panel and the diagnostics window use — the same
//! atomic writes, link rewriting and schedule reconciliation included.
//!
//! **Deny by default.** With `--automation-read-root`/`--automation-write-root`, every vault and
//! note a command opens, reads, writes, renames or exports goes through
//! [`craft_mcp::automation::Roots`]; a kind left out grants none of its access. No roots at all
//! means no confinement (the user's own app).

use std::path::{Path, PathBuf};

use craft_mcp::automation::{Access, Roots};
use serde_json::{Value, json};

use super::catalogue;
use crate::vault_state::VaultState;

/// Most notes `vault.inspect` lists unless asked for more (0 lists all).
pub const INSPECT_DEFAULT: u64 = 500;
/// Most notes `vault.search` returns unless asked for more (0 returns all).
pub const SEARCH_DEFAULT: u64 = 50;

/// A headless session: one vault (its tree, index, open note and schedule), the automation roots
/// and the commands on top of them.
pub struct Session {
    vault: VaultState,
    roots: Option<Roots>,
}

impl Session {
    pub fn new(roots: Option<Roots>) -> Self {
        Self { vault: VaultState::default(), roots }
    }

    /// Where this session's calls go, for the MCP server's instructions.
    pub fn describe(&self) -> String {
        match &self.roots {
            None => "headless".to_string(),
            Some(roots) => format!("headless, files confined: {}", roots.describe()),
        }
    }

    /// One control-channel method, as the running app answers them: `command.execute`,
    /// `command.list` and `app.info`.
    pub fn call(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "command.execute" => {
                let command = params.get("command").and_then(Value::as_str).ok_or("command.execute: missing string `command`")?;
                let args = params.get("params").cloned().unwrap_or(Value::Null);
                self.exec(command, &args)
            }
            "command.list" => {
                let filter = params.get("filter").and_then(Value::as_str);
                let brief = params.get("brief").and_then(Value::as_bool).unwrap_or(false);
                Ok(Value::Array(catalogue::search(filter, brief)))
            }
            "app.info" => Ok(json!({
                "id": "localnotebook",
                "name": "Local Notebook",
                "version": env!("CARGO_PKG_VERSION"),
                "state": state(&self.vault),
            })),
            other => Err(format!("unknown method `{other}`: this app answers command.execute, command.list and app.info")),
        }
    }

    /// Run one command by id.
    pub fn exec(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let Some(spec) = catalogue::spec(id) else {
            return Err(format!("unknown command `{id}`: command_list lists what this app has"));
        };
        if spec.app_only {
            return Err(format!(
                "`{id}` runs in the window: start the app with `localnotebook --control PORT` and use `mcp --connect PORT`, or run it from the palette"
            ));
        }
        let args = catalogue::arguments(id, params)?;
        let text =
            |key: &str| -> Result<&str, String> { args.get(key).and_then(Value::as_str).ok_or_else(|| format!("`{id}`: `{key}` must be a string")) };
        let optional = |key: &str| args.get(key).and_then(Value::as_str);
        let vault = &mut self.vault;
        let roots = self.roots.as_ref();
        match id {
            "file.open" => open_vault(vault, roots, text("path")?),
            "file.new" => new_note(vault, roots, text("name")?, optional("text"), optional("folder")),
            "file.rename" => rename_note(vault, roots, optional("path"), text("name")?),
            "file.save" => save(vault, roots),
            "file.export_html" => export(vault, roots, optional("note"), text("path")?, Format::Html),
            "file.export_pdf" => export(vault, roots, optional("note"), text("path")?, Format::Pdf),
            "note.open" => open_note(vault, roots, text("path")?),
            "note.read" => read_note(vault, roots, optional("path")),
            "note.write" => write_note(vault, roots, optional("path"), text("text")?),
            "vault.inspect" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(INSPECT_DEFAULT);
                Ok(inspect(vault, limit))
            }
            "vault.search" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(SEARCH_DEFAULT);
                let regex = args.get("regex").and_then(Value::as_bool).unwrap_or(false);
                search(vault, text("query")?, regex, limit)
            }
            "vault.diagnostics" => Ok(diagnostics(vault)),
            // Every other command is `appOnly` and was refused above; a new one lands here until
            // its arm exists, which the "every command runs somewhere" test catches.
            _ => Err(format!("`{id}` has no headless implementation")),
        }
    }
}

/// Which file `file.export_*` writes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Html,
    Pdf,
}

impl Format {
    fn id(self) -> &'static str {
        match self {
            Format::Html => "html",
            Format::Pdf => "pdf",
        }
    }
}

/// The vault as replies describe it: its folder, how many notes it holds and which one is open.
pub fn state(vault: &VaultState) -> Value {
    json!({
        "root": vault.root.as_deref().map(display),
        "notes": vault.files.len(),
        "active": vault.active.as_deref().map(|path| relative(vault, path)),
        "dirty": vault.dirty,
    })
}

/// Open a vault (`file.open`): scans the tree, reads every note and builds the index.
pub fn open_vault(vault: &mut VaultState, roots: Option<&Roots>, path: &str) -> Result<Value, String> {
    let asked = clean(path);
    check(roots, &asked, Access::Read)?;
    let real = resolve(&asked)?;
    if !real.is_dir() {
        return Err(format!("`{}` is not a folder: give the vault's folder", display(&real)));
    }
    let shown = display(&real);
    vault.open(real).map_err(|e| format!("{shown}: {e}"))?;
    vault.open_first_note();
    Ok(json!({"state": state(vault)}))
}

/// Open a note in the editor (`note.open`): its text becomes the buffer.
pub fn open_note(vault: &mut VaultState, roots: Option<&Roots>, asked: &str) -> Result<Value, String> {
    let path = note_path(vault, asked)?;
    check(roots, &display(&path), Access::Read)?;
    if vault.active.as_deref() != Some(path.as_path()) {
        vault.open_note(&path);
    }
    if vault.active.as_deref() != Some(path.as_path()) {
        return Err(vault.message.clone().unwrap_or_else(|| format!("`{}` could not be opened", display(&path))));
    }
    Ok(json!({"path": relative(vault, &path), "state": state(vault)}))
}

/// The text of a note (`note.read`): the open note's buffer (edits included) or a note from disk.
pub fn read_note(vault: &VaultState, roots: Option<&Roots>, asked: Option<&str>) -> Result<Value, String> {
    let (path, text) = match asked {
        Some(asked) => {
            let path = note_path(vault, asked)?;
            check(roots, &display(&path), Access::Read)?;
            let text = localnotebook_vault::read_note(&path).map_err(|e| format!("{}: {e}", display(&path)))?;
            (path, text)
        }
        None => {
            let path = vault.active.clone().ok_or("no note is open: `note.open`, or give `path`")?;
            (path, vault.buffer.clone())
        }
    };
    Ok(json!({"path": relative(vault, &path), "bytes": text.len(), "lines": text.lines().count(), "text": text}))
}

/// Replace a note's whole text and write it (`note.write`), atomically, as the editor's save does.
pub fn write_note(vault: &mut VaultState, roots: Option<&Roots>, asked: Option<&str>, text: &str) -> Result<Value, String> {
    let path = match asked {
        Some(asked) => note_path(vault, asked)?,
        None => vault.active.clone().ok_or("no note is open: `note.open`, or give `path`")?,
    };
    if vault.active.as_deref() != Some(path.as_path()) {
        // Opening it reads the file first; editing it afterwards is the write.
        check(roots, &display(&path), Access::Read)?;
        vault.open_note(&path);
        if vault.active.as_deref() != Some(path.as_path()) {
            return Err(vault.message.clone().unwrap_or_else(|| format!("`{}` could not be opened", display(&path))));
        }
    }
    check(roots, &display(&path), Access::Write)?;
    vault.replace_buffer(text.to_string());
    if vault.buffer != text {
        // The editor ignores an empty buffer (there is nothing to write into a note), so say why.
        return Err(format!("`{}` could not take the text (the editor keeps a note non-empty)", display(&path)));
    }
    vault.save().map_err(|e| format!("{}: {e}", display(&path)))?;
    Ok(json!({"path": relative(vault, &path), "bytes": text.len(), "state": state(vault)}))
}

/// Create a note and open it (`file.new`), as the New note dialog does with its template.
pub fn new_note(vault: &mut VaultState, roots: Option<&Roots>, name: &str, text: Option<&str>, folder: Option<&str>) -> Result<Value, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("`name` is empty: a note needs a name".into());
    }
    let root = vault.root.clone().ok_or("no vault is open: run `file.open` first")?;
    let folder = match folder {
        Some(folder) => root.join(clean(folder)),
        None => root.clone(),
    };
    let target = folder.join(if name.to_ascii_lowercase().ends_with(".md") { name.to_string() } else { format!("{name}.md") });
    check(roots, &display(&target), Access::Write)?;
    let content = match text {
        Some(text) => text.to_string(),
        None => crate::templates::expand_builtin("blank", name, chrono::Local::now()),
    };
    let path = vault.create_note_with(name, &content, Some(&folder)).map_err(|e| format!("{}: {e}", display(&target)))?;
    Ok(json!({"path": relative(vault, &path), "state": state(vault)}))
}

/// Rename a note (`file.rename`), rewriting every `[[link]]` that pointed at it — so the whole
/// vault must be writable: the operation can rewrite any note.
pub fn rename_note(vault: &mut VaultState, roots: Option<&Roots>, asked: Option<&str>, name: &str) -> Result<Value, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("`name` is empty: a rename needs a name".into());
    }
    let root = vault.root.clone().ok_or("no vault is open: run `file.open` first")?;
    check(roots, &display(&root), Access::Write)?;
    let path = match asked {
        Some(asked) => note_path(vault, asked)?,
        None => vault.active.clone().ok_or("no note is open: `note.open`, or give `path`")?,
    };
    if vault.active.as_deref() != Some(path.as_path()) {
        check(roots, &display(&path), Access::Read)?;
        vault.open_note(&path);
    }
    let links = vault.rename_active(name).map_err(|e| format!("{name}: {e}"))?;
    Ok(json!({"name": name, "links": links, "state": state(vault)}))
}

/// Write the open note (`file.save`); a note nobody edited is not written again.
pub fn save(vault: &mut VaultState, roots: Option<&Roots>) -> Result<Value, String> {
    let path = vault.active.clone().ok_or("no note is open: `note.open` first")?;
    check(roots, &display(&path), Access::Write)?;
    vault.save().map_err(|e| format!("{}: {e}", display(&path)))?;
    Ok(json!({"path": relative(vault, &path), "state": state(vault)}))
}

/// Write the open note as HTML or PDF (`file.export_html` / `file.export_pdf`).
pub fn export(vault: &mut VaultState, roots: Option<&Roots>, note: Option<&str>, path: &str, format: Format) -> Result<Value, String> {
    if let Some(note) = note {
        open_note(vault, roots, note)?;
    } else if vault.active.is_none() {
        return Err("no note is open: `note.open`, or give `note`".into());
    }
    let asked = clean(path);
    check(roots, &asked, Access::Write)?;
    let target = PathBuf::from(&asked);
    match format {
        Format::Html => {
            let copied = crate::app::write_html(vault, &target, crate::i18n::code())?;
            let bytes = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
            Ok(json!({"path": asked, "format": format.id(), "bytes": bytes, "images": copied}))
        }
        Format::Pdf => {
            let report = crate::app::write_pdf(vault, &target)?;
            let bytes = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
            Ok(json!({"path": asked, "format": format.id(), "bytes": bytes, "pages": report.pages}))
        }
    }
}

/// The vault's notes, for an agent to pick a path (`vault.inspect`).
pub fn inspect(vault: &VaultState, limit: u64) -> Value {
    let most = usize::try_from(limit).unwrap_or(usize::MAX);
    let taken = if most == 0 { vault.files.len() } else { most.min(vault.files.len()) };
    let files: Vec<Value> = vault.files.iter().take(taken).map(|note| json!({"path": note.relative_path, "name": note.name})).collect();
    let mut tags: Vec<(String, usize)> = vault.tag_counts();
    tags.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    tags.truncate(50);
    let tags: Vec<Value> = tags.into_iter().map(|(tag, count)| json!({"tag": tag, "count": count})).collect();
    let mut out = state(vault);
    out["truncated"] = json!(taken < vault.files.len());
    out["files"] = Value::Array(files);
    out["tags"] = Value::Array(tags);
    out
}

/// Run the search panel's engine on `query` (`vault.search`): the same matches and order the panel
/// shows, without waiting for the typing to settle.
pub fn search(vault: &mut VaultState, query: &str, regex: bool, limit: u64) -> Result<Value, String> {
    if vault.root.is_none() {
        return Err("no vault is open: run `file.open` first".into());
    }
    vault.search.query = query.to_string();
    vault.search.regex = regex;
    vault.run_search_now();
    if vault.search.invalid {
        return Err(format!("`{query}` is not a valid regular expression"));
    }
    let most = usize::try_from(limit).unwrap_or(usize::MAX);
    let total = vault.search.results.len();
    let taken = if most == 0 { total } else { most.min(total) };
    let results: Vec<Value> = vault
        .search
        .results
        .iter()
        .take(taken)
        .map(|hit| {
            let matches: Vec<Value> = hit.matches.iter().map(|m| json!({"line": m.line, "text": m.text})).collect();
            json!({"path": hit.relative_path, "name": hit.name, "score": hit.score, "matches": matches})
        })
        .collect();
    Ok(json!({"query": query, "regex": regex, "count": total, "truncated": taken < total, "results": results}))
}

/// Broken links, orphans and duplicated names (`vault.diagnostics`), from the index in hand.
pub fn diagnostics(vault: &VaultState) -> Value {
    if vault.root.is_none() {
        return json!({"clean": true, "brokenLinks": [], "orphans": [], "duplicates": []});
    }
    let found = vault.diagnostics();
    let broken: Vec<Value> = found.broken_links.iter().map(|link| json!({"target": link.target, "from": link.from})).collect();
    let duplicates: Vec<Value> = found.duplicates.iter().map(|dup| json!({"name": dup.name, "files": dup.files})).collect();
    json!({"clean": found.is_clean(), "brokenLinks": broken, "orphans": found.orphans, "duplicates": duplicates})
}

/// The file a note reference names: an absolute path, a vault-relative path (`Pasta/Nota.md`, the
/// `.md` optional) or a bare note name (refused when two notes share it).
fn note_path(vault: &VaultState, asked: &str) -> Result<PathBuf, String> {
    let text = clean(asked);
    if text.is_empty() {
        return Err("`path` is empty: a note needs one".into());
    }
    let as_given = PathBuf::from(&text);
    if as_given.is_absolute() {
        return Ok(as_given);
    }
    let root = vault.root.as_deref().ok_or("no vault is open: run `file.open` first")?;
    let with_md = if text.to_ascii_lowercase().ends_with(".md") { text.clone() } else { format!("{text}.md") };
    let candidate = root.join(&with_md);
    if candidate.is_file() {
        return Ok(candidate);
    }
    if !text.contains('/') && !text.contains('\\') {
        let stem = text.strip_suffix(".md").unwrap_or(&text);
        let mut found = vault.files.iter().filter(|note| note.name.eq_ignore_ascii_case(stem));
        match (found.next(), found.next()) {
            (Some(one), None) => return Ok(one.path.clone()),
            (Some(one), Some(two)) => {
                return Err(format!(
                    "`{text}` names more than one note ({} and {}): give the vault-relative path",
                    one.relative_path, two.relative_path
                ));
            }
            _ => {}
        }
    }
    Err(format!("no note `{text}` in the vault: vault.inspect lists them"))
}

/// A note's path as replies show it: vault-relative with `/` separators.
fn relative(vault: &VaultState, path: &Path) -> String {
    match vault.root.as_deref() {
        Some(root) => localnotebook_vault::relative_display(root, path),
        None => display(path),
    }
}

/// `path` as given, cleaned of the quotes a "Copy as path" paste brings.
fn clean(path: &str) -> String {
    let text = path.trim();
    text.strip_prefix('"').and_then(|r| r.strip_suffix('"')).unwrap_or(text).trim().to_string()
}

/// Refuse `path` when it is outside its kind's automation roots (no roots: nothing is confined).
fn check(roots: Option<&Roots>, path: &str, access: Access) -> Result<(), String> {
    if path.is_empty() {
        return Err("empty path".into());
    }
    match roots {
        None => Ok(()),
        Some(roots) => roots.check(path, access),
    }
}

/// `path` made absolute with its links followed, without Windows' `\\?\` prefix.
fn resolve(path: &str) -> Result<PathBuf, String> {
    let real = std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))?;
    Ok(PathBuf::from(display(&real)))
}

/// A path as replies and messages show it, without Windows' `\\?\` prefix.
fn display(path: &Path) -> String {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_string(),
        _ => text.into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vault with two notes and a link between them; returns its folder.
    fn vault(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("localnotebook-auto-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(dir.join("Pasta")).expect("scratch");
        std::fs::write(dir.join("Alfa.md"), "# Alfa\n\nVeja [[Beta]], [[NaoExiste]] e #projeto.\n").expect("note");
        std::fs::write(dir.join("Pasta/Beta.md"), "# Beta\n\nUm cartão?\n\nQ: pergunta\nA: resposta\n").expect("note");
        dir
    }

    fn exec(s: &mut Session, id: &str, params: Value) -> Result<Value, String> {
        s.exec(id, &params)
    }

    #[test]
    fn a_vault_opens_and_its_notes_read_write_and_rename() {
        let dir = vault("rwin");
        let mut s = Session::new(None);
        let opened = exec(&mut s, "file.open", json!({"path": dir.to_string_lossy()})).unwrap();
        assert_eq!(opened["state"]["notes"], 2);
        assert_eq!(opened["state"]["active"], "Alfa.md", "the first note opens");

        let read = exec(&mut s, "note.read", json!({"path": "Beta"})).unwrap();
        assert!(read["text"].as_str().unwrap_or_default().contains("# Beta"), "{read}");
        assert_eq!(read["path"], "Pasta/Beta.md");

        let written = exec(&mut s, "note.write", json!({"path": "Pasta/Beta.md", "text": "# Beta\n\nreescrita\n"})).unwrap();
        assert_eq!(written["state"]["dirty"], false);
        let from_disk = std::fs::read_to_string(dir.join("Pasta/Beta.md")).unwrap();
        assert_eq!(from_disk, "# Beta\n\nreescrita\n", "written atomically through the app's save");

        // A new note lands in the vault, with the dialog's blank template.
        let made = exec(&mut s, "file.new", json!({"name": "Gama"})).unwrap();
        assert_eq!(made["path"], "Gama.md");
        assert!(dir.join("Gama.md").is_file());
        assert_eq!(exec(&mut s, "vault.inspect", json!({})).expect("inspect")["notes"], 3);

        // Renaming rewrites the links that pointed at the note.
        let renamed = exec(&mut s, "file.rename", json!({"path": "Pasta/Beta.md", "name": "Delta"})).unwrap();
        assert_eq!(renamed["links"], 1, "{renamed}");
        let alfa = std::fs::read_to_string(dir.join("Alfa.md")).unwrap();
        assert!(alfa.contains("[[Delta]]"), "{alfa}");
        assert!(dir.join("Pasta/Delta.md").is_file());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn search_diagnostics_and_inspect_are_the_panels_data() {
        let dir = vault("search");
        let mut s = Session::new(None);
        exec(&mut s, "file.open", json!({"path": dir.to_string_lossy()})).unwrap();
        let found = exec(&mut s, "vault.search", json!({"query": "pergunta"})).unwrap();
        assert_eq!(found["count"], 1);
        assert_eq!(found["results"][0]["path"], "Pasta/Beta.md");
        assert!(
            exec(&mut s, "vault.search", json!({"query": "projeto"})).unwrap()["results"][0]["matches"].as_array().is_some_and(|m| !m.is_empty())
        );
        assert!(exec(&mut s, "vault.search", json!({"query": "[", "regex": true})).unwrap_err().contains("regular expression"));

        let inspected = exec(&mut s, "vault.inspect", json!({"limit": 1})).unwrap();
        assert_eq!(inspected["truncated"], true);
        assert_eq!(inspected["files"].as_array().map(Vec::len), Some(1));
        assert!(inspected["tags"].as_array().is_some_and(|t| t.iter().any(|tag| tag["tag"] == "projeto")), "{inspected}");

        let diag = exec(&mut s, "vault.diagnostics", json!({})).unwrap();
        assert_eq!(diag["clean"], false, "[[NaoExiste]] is a broken link: {diag}");
        assert!(diag["brokenLinks"].as_array().is_some_and(|b| b.iter().any(|link| link["target"] == "NaoExiste")), "{diag}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_path_outside_the_roots_is_refused() {
        let inside = vault("inside");
        let outside = vault("outside");
        let roots = Roots::new(&[inside.as_path()], &[inside.as_path()]).unwrap().unwrap();
        let mut s = Session::new(Some(roots));
        let e = exec(&mut s, "file.open", json!({"path": outside.to_string_lossy()})).unwrap_err();
        assert!(e.contains("automation path rejected") && e.contains("outside the read root"), "{e}");
        exec(&mut s, "file.open", json!({"path": inside.to_string_lossy()})).unwrap();
        let e = exec(&mut s, "file.export_html", json!({"path": outside.join("a.html").to_string_lossy()})).unwrap_err();
        assert!(e.contains("outside the write root"), "{e}");
        exec(&mut s, "file.export_html", json!({"path": inside.join("a.html").to_string_lossy()})).unwrap();
        assert!(inside.join("a.html").is_file());

        // Reading is granted, writing is not: a kind left out grants none of its access.
        let read_only = Roots::new(&[inside.as_path()], &[]).unwrap().unwrap();
        let mut s = Session::new(Some(read_only));
        exec(&mut s, "file.open", json!({"path": inside.to_string_lossy()})).unwrap();
        let e = exec(&mut s, "note.write", json!({"path": "Alfa.md", "text": "x\\n"})).unwrap_err();
        assert!(e.contains("write authority is absent"), "{e}");
        std::fs::remove_dir_all(inside).ok();
        std::fs::remove_dir_all(outside).ok();
    }

    #[test]
    fn window_only_commands_say_where_they_run() {
        let mut s = Session::new(None);
        for id in ["view.sidebar", "review.vault", "ai.chat", "file.exit"] {
            let e = exec(&mut s, id, json!({})).unwrap_err();
            assert!(e.contains("runs in the window") && e.contains("--control"), "{id}: {e}");
        }
        assert!(exec(&mut s, "note.delete", json!({})).is_err(), "an unknown id is refused");
        assert!(exec(&mut s, "file.save", json!({"force": true})).unwrap_err().contains("unknown parameter `force`"));
    }

    #[test]
    fn every_command_runs_somewhere() {
        let mut s = Session::new(None);
        for spec in catalogue::SPECS {
            let result = exec(&mut s, spec.id, json!({}));
            if spec.app_only {
                assert!(result.as_ref().err().is_some_and(|e| e.contains("runs in the window")), "{}", spec.id);
            } else {
                assert!(!result.as_ref().err().is_some_and(|e| e.contains("no headless implementation")), "{}: {result:?}", spec.id);
            }
        }
    }

    #[test]
    fn the_control_methods_answer_command_list_and_app_info() {
        let mut s = Session::new(None);
        assert_eq!(s.call("command.list", &json!({"filter": "vault."})).unwrap().as_array().map(Vec::len), Some(3));
        let info = s.call("app.info", &json!({})).unwrap();
        assert_eq!(info["id"], "localnotebook");
        assert!(s.call("engine.execute", &json!({})).unwrap_err().contains("unknown method"));
    }
}
