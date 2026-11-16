//! Local Obsidian - notes, wikilinks and attachments in a plain folder (Rust + egui, no account,
//! no telemetry, no third-party servers).
//!
//! Usage: `localobsidian [--version] [--help]`. A born app adds its own arguments here.
//!
//! On Windows, release builds are GUI-subsystem programs: launching the app opens no console
//! window. Their `--help` / `--version` output then has no console to go to; use a debug build or
//! a terminal that captures the child's output.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod ai_ui;
mod app;
mod calendar;
mod completions;
mod connections;
mod diagnostics_ui;
mod editor;
mod editor_keys;
mod fonts;
mod history;
mod i18n;
mod images;
mod opener;
mod panels;
mod pickers;
mod prefs;
mod review_ui;
mod sidebar;
mod suite_entry;
mod templates;
mod theme;
mod vault_state;
mod watcher;

#[cfg(test)]
mod ui_tests;

use app::LocalApp;
use suite_entry::APP;

/// Reverse-DNS app id: the Wayland app id, the `.desktop` file and the hicolor icon name. It must
/// match `packaging/linux/com.localobsidian.app.desktop` and the MSI's icon/registry names.
const APP_ID: &str = "com.localobsidian.app";

/// The window, taskbar and Dock icon, committed by `packaging/icons.sh`. The 256 px hicolor entry
/// is the runtime icon; the ICO built from the same SVG is the executable's (see `build.rs`).
fn app_icon() -> egui::IconData {
    let png: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/com.localobsidian.app.png");
    eframe::icon_data::from_png_bytes(png).unwrap_or_default()
}

const HELP: &str = "\
localobsidian - notes, wikilinks and attachments in a plain folder

USAGE: localobsidian [OPTIONS]

OPTIONS:
  -h, --help       print this help and exit
      --version    print the version and exit

The interface language follows the system's; the About window (Help menu) lets you pick another
one and checks for updates. Updates install from the GitHub releases of Anon5T4R/Local-Obsidian.
";

/// `1440x900` → the two numbers, for the fixed-size QA window.
fn parse_size(size: &str) -> Option<[f32; 2]> {
    let (width, height) = size.split_once('x')?;
    let width: f32 = width.trim().parse().ok()?;
    let height: f32 = height.trim().parse().ok()?;
    (width >= 320.0 && height >= 240.0).then_some([width, height])
}

fn main() -> eframe::Result {
    // Windows: the update helper is a copy of this program started with `--finish-update`. Nothing
    // else runs in that mode, and nothing at all happens off Windows.
    if let Some(result) = craft_update::self_update::finish(&APP, &std::env::args_os().skip(1).collect::<Vec<_>>()) {
        std::process::exit(i32::from(result.is_err()));
    }
    // Nothing but these flags is accepted: a born app grows its own arguments here (a library
    // directory, files to import, .) and rejects what it does not know.
    let mut unknown = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--version" => {
                println!("{} {}", APP.id, env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(());
            }
            other => unknown.push(other.to_string()),
        }
    }
    if !unknown.is_empty() {
        for option in &unknown {
            eprintln!("{}: unknown option `{option}`", APP.id);
        }
        eprintln!("\n{HELP}");
        std::process::exit(2);
    }

    // Who draws the window frame: this is the suite's one decision (the desktop on Windows, macOS
    // and X11; on Wayland, the compositor when it offers server-side decorations, the app on GNOME's
    // Mutter). `LOCALOBSIDIAN_TITLEBAR=app|system` overrides it.
    let choice = craft_window::Choice::probe(craft_window::Prefix::new("LOCALOBSIDIAN"));
    let mut viewport = craft_window::decorate(
        egui::ViewportBuilder::default().with_title(APP.name).with_min_inner_size([720.0, 440.0]).with_icon(app_icon()).with_app_id(APP_ID),
        choice.frame(),
    );
    // A fixed window for the QA capture, so two runs can be compared pixel by pixel.
    if let Some(size) = std::env::var("LOCALOBSIDIAN_QA_SIZE").ok().and_then(|size| parse_size(&size)) {
        viewport = viewport.with_inner_size(size);
    }
    let frame = choice.into_frame();
    eframe::run_native(APP.name, eframe::NativeOptions { viewport, ..Default::default() }, Box::new(move |cc| Ok(Box::new(LocalApp::new(cc, frame)))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_id_is_reverse_dns_and_matches_the_desktop_file_name() {
        assert_eq!(APP_ID, "com.localobsidian.app");
        assert_eq!(APP_ID, format!("com.{}.app", APP.id));
    }

    #[test]
    fn help_documents_the_flags_the_parser_accepts() {
        for flag in ["-h", "--help", "--version"] {
            assert!(HELP.contains(flag), "{flag} is parsed but not documented");
        }
    }
}
