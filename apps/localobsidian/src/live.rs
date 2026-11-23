//! The window's side of the control channel (`localobsidian --control PORT`): the same commands the
//! headless session runs, executed on the vault the window has open, so an agent's `note.write`
//! writes the note the user is reading.
//!
//! Each frame [`LocalApp::pump_control`] answers what arrived. The commands are the shared ones
//! (`automation::session`) over the app's own `VaultState`; the window follows (the sidebar, the
//! editor, the reading view and the status strip read that state every frame).

use craft_mcp::automation::Roots;
use serde_json::{Map, Value, json};

use crate::app::LocalApp;
use crate::automation::catalogue;
use crate::automation::session::{self, Format};
use crate::vault_state::VaultState;

impl LocalApp {
    /// Answer the control channel's pending requests with this frame's state, if the app was
    /// started with `--control`.
    pub(crate) fn pump_control(&mut self, ctx: &egui::Context) {
        // Taken out so the dispatch can borrow the whole app; the channel goes back afterwards.
        let Some(mut control) = self.control.take() else { return };
        control.pump(|method, params| self.live_call(ctx, method, params));
        self.control = Some(control);
        // A request may have arrived while the window was idle: keep answering while the channel
        // is open (it exists only in the `--control` mode).
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    /// One control-channel method of the running app.
    fn live_call(&mut self, ctx: &egui::Context, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "command.execute" => {
                let id = params.get("command").and_then(Value::as_str).ok_or("command.execute: missing string `command`")?;
                let args = params.get("params").cloned().unwrap_or(Value::Null);
                self.live_exec(ctx, id, &args)
            }
            "command.list" => {
                let filter = params.get("filter").and_then(Value::as_str);
                let brief = params.get("brief").and_then(Value::as_bool).unwrap_or(false);
                Ok(Value::Array(catalogue::search(filter, brief)))
            }
            "app.info" => Ok(json!({
                "id": "localobsidian",
                "name": "Local Obsidian",
                "version": env!("CARGO_PKG_VERSION"),
                "backend": "window",
                "state": session::state(&self.vault),
            })),
            other => Err(format!("unknown method `{other}`: this app answers command.execute, command.list and app.info")),
        }
    }

    /// Run one command on the open vault, then apply the window's effects.
    fn live_exec(&mut self, ctx: &egui::Context, id: &str, params: &Value) -> Result<Value, String> {
        if !catalogue::has(id) {
            return Err(format!("unknown command `{id}`: command_list lists what this app has"));
        }
        let args = catalogue::arguments(id, params)?;
        match id {
            "file.open" => {
                let path = arg(&args, id, "path")?;
                let out = self.vault_with(|vault, roots| session::open_vault(vault, roots, path))?;
                // The window's own open also watches the folder and remembers it.
                self.after_open_vault();
                Ok(out)
            }
            "file.new" => {
                let (name, text, folder) = (arg(&args, id, "name")?, opt(&args, "text"), opt(&args, "folder"));
                self.vault_with(|vault, roots| session::new_note(vault, roots, name, text, folder))
            }
            "file.rename" => {
                let (path, name) = (opt(&args, "path"), arg(&args, id, "name")?);
                self.vault_with(|vault, roots| session::rename_note(vault, roots, path, name))
            }
            "file.save" => self.vault_with(session::save),
            "file.export_html" => {
                let (note, path) = (opt(&args, "note"), arg(&args, id, "path")?);
                self.vault_with(|vault, roots| session::export(vault, roots, note, path, Format::Html))
            }
            "file.export_pdf" => {
                let (note, path) = (opt(&args, "note"), arg(&args, id, "path")?);
                self.vault_with(|vault, roots| session::export(vault, roots, note, path, Format::Pdf))
            }
            "note.open" => {
                let path = arg(&args, id, "path")?;
                self.vault_with(|vault, roots| session::open_note(vault, roots, path))
            }
            "note.read" => self.vault_read(|vault, roots| session::read_note(vault, roots, opt(&args, "path"))),
            "note.write" => {
                let (path, text) = (opt(&args, "path"), arg(&args, id, "text")?);
                self.vault_with(|vault, roots| session::write_note(vault, roots, path, text))
            }
            "vault.inspect" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(session::INSPECT_DEFAULT);
                Ok(session::inspect(&self.vault, limit))
            }
            "vault.search" => {
                let (query, regex, limit) = (
                    arg(&args, id, "query")?,
                    args.get("regex").and_then(Value::as_bool).unwrap_or(false),
                    args.get("limit").and_then(Value::as_u64).unwrap_or(session::SEARCH_DEFAULT),
                );
                self.vault_with(|vault, _| session::search(vault, query, regex, limit))
            }
            "vault.diagnostics" => Ok(session::diagnostics(&self.vault)),
            _ => {
                // What is left needs the window: the viewport, the panels, the review session, the
                // AI chat, the updater, exit. The palette's own dispatch, so the behaviour cannot
                // drift.
                self.run_command(ctx, id);
                Ok(json!({"command": id, "state": session::state(&self.vault)}))
            }
        }
    }

    /// Run `f` on the app's vault with the automation roots in hand: the fields are borrowed apart
    /// so both are reachable.
    fn vault_with<R>(&mut self, f: impl FnOnce(&mut VaultState, Option<&Roots>) -> R) -> R {
        let Self { vault, roots, .. } = self;
        f(vault, roots.as_ref())
    }

    /// The same, for a command that only reads the vault.
    fn vault_read<R>(&self, f: impl FnOnce(&VaultState, Option<&Roots>) -> R) -> R {
        f(&self.vault, self.roots.as_ref())
    }
}

/// A required string parameter.
fn arg<'a>(args: &'a Map<String, Value>, id: &str, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| format!("`{id}`: `{key}` must be a string"))
}

/// An optional string parameter.
fn opt<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}
