//! Handing a file or a folder to the desktop.
//!
//! The vault is a folder of files, and a `.pdf` in it belongs to whatever the user opens PDFs with -
//! not to this app. The commands are the platform's own (`explorer`, `open`, `xdg-open`); building
//! them is a pure function so the shape is testable without spawning anything.

use std::path::Path;

/// The program and the arguments that open `path` with the desktop's handler.
pub fn open_command(path: &Path) -> (&'static str, Vec<String>) {
    let path = path.to_string_lossy().to_string();
    if cfg!(windows) {
        // `start` needs its empty title argument, or a quoted path is taken as one.
        ("cmd", vec!["/C".into(), "start".into(), String::new(), path])
    } else if cfg!(target_os = "macos") {
        ("open", vec![path])
    } else {
        ("xdg-open", vec![path])
    }
}

/// The program and the arguments that show `path` in the file manager, with the folder selected.
pub fn reveal_command(path: &Path) -> (&'static str, Vec<String>) {
    let path = path.to_string_lossy().to_string();
    if cfg!(windows) {
        // `explorer` wants `/select,<path>` with no space for a file; for a folder it opens it.
        if path_is_folder(path.as_ref()) { ("explorer", vec![path]) } else { ("explorer", vec![format!("/select,{path}")]) }
    } else if cfg!(target_os = "macos") {
        ("open", vec!["-R".into(), path])
    } else {
        // There is no portable "select" on Linux: opening the folder is what every file manager has.
        ("xdg-open", vec![folder_of(path.as_ref())])
    }
}

/// Opens the file with the system's handler. A failure is returned, not crashed on.
pub fn open(path: &Path) -> Result<(), String> {
    spawn(open_command(path))
}

/// Shows the path in the file manager.
pub fn reveal(path: &Path) -> Result<(), String> {
    spawn(reveal_command(path))
}

fn spawn((program, args): (&'static str, Vec<String>)) -> Result<(), String> {
    // `craft_update::process::command` is the suite's spawn: no console flash on Windows, and the
    // AppImage's environment is cleaned so the system handler is not handed our own libraries.
    craft_update::process::command(program).args(&args).spawn().map(|_| ()).map_err(|error| format!("{program}: {error}"))
}

fn path_is_folder(path: &Path) -> bool {
    path.is_dir()
}

/// The folder a path lives in (the path itself when it is one).
fn folder_of(path: &Path) -> String {
    if path.is_dir() {
        path.to_string_lossy().to_string()
    } else {
        path.parent().map(|parent| parent.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string_lossy().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_a_file_uses_the_platforms_handler() {
        let (program, args) = open_command(Path::new("/v/protocolo.pdf"));
        assert!(args.iter().any(|arg| arg.ends_with("protocolo.pdf")), "{program} {args:?}");
        #[cfg(windows)]
        {
            assert_eq!(program, "cmd");
            assert!(args.iter().any(|arg| arg == "start"));
        }
        #[cfg(target_os = "macos")]
        assert_eq!(program, "open");
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(program, "xdg-open");
    }

    #[test]
    fn revealing_a_folder_shows_that_folder() {
        // A real folder: the shape of the command depends on whether the path is a folder, and a
        // made-up path would only test the branch `false`.
        let root = std::env::temp_dir().join(format!("localobsidian-reveal-{}", std::process::id()));
        let folder = root.join("2. Medicina");
        std::fs::create_dir_all(&folder).expect("scratch dir");

        let (program, args) = reveal_command(&folder);
        let asked = folder.to_string_lossy().to_string();
        assert!(args.iter().any(|arg| arg.contains(&asked)), "{program} {args:?}");
        #[cfg(windows)]
        assert_eq!(program, "explorer");
        #[cfg(target_os = "macos")]
        assert_eq!(args.first().map(String::as_str), Some("-R"));
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(program, "xdg-open");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn revealing_a_file_selects_it_where_the_desktop_can_and_opens_its_folder_where_it_cannot() {
        let root = std::env::temp_dir().join(format!("localobsidian-reveal-file-{}", std::process::id()));
        let file = root.join("protocolo.pdf");
        std::fs::create_dir_all(&root).expect("scratch dir");
        std::fs::write(&file, "%PDF-1.4").expect("write");

        let (program, args) = reveal_command(&file);
        #[cfg(windows)]
        {
            assert_eq!(program, "explorer");
            assert!(args.first().is_some_and(|arg| arg.starts_with("/select,")), "explorer selects the file: {args:?}");
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(program, "open");
            assert_eq!(args.first().map(String::as_str), Some("-R"));
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            // No portable "select" on Linux: the folder is what every file manager opens.
            assert_eq!(program, "xdg-open");
            let asked = root.to_string_lossy().to_string();
            assert!(args.iter().any(|arg| arg.contains(&asked)), "the folder is what opens: {args:?}");
        }
        let _ = (program, args);

        let _ = std::fs::remove_dir_all(&root);
    }
}
