//! Writing a note without ever leaving it half-written.
//!
//! `fs::write` opens the real file and truncates it before the new bytes go in: a crash, a power
//! cut or a killed process inside that window leaves the note empty or cut in half. Autosave runs
//! while you type, so that window is open a lot. The content goes to a dotfile beside the note
//! first, then a rename puts it in place - atomic on NTFS and ext4, so the note is either the old
//! one or the new one, never a fragment.
//!
//! The dotfile prefix is not cosmetic: the tree hides entries that start with `.`, so staging a
//! save never flickers a phantom note into the sidebar.

use std::io;
use std::path::{Path, PathBuf};

/// Where the new content is staged, beside the note it is replacing.
pub fn temp_path_for(path: &Path) -> PathBuf {
    let name = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
    path.with_file_name(format!(".{name}.saving"))
}

/// The note's text. Bytes that are not UTF-8 are refused instead of being replaced: a vault holds
/// the user's notes, and silently swapping characters out of one is worse than not opening it.
pub fn read_note(path: &Path) -> io::Result<String> {
    std::fs::read_to_string(path)
}

/// Content first to a temp file, then a rename over the target.
///
/// If the rename keeps failing (a sync client or an antivirus holding the file on Windows), it
/// falls back to writing in place: that is what the app would have done anyway, so the fallback can
/// only match the old behaviour - and a save that fails outright is reported to the caller instead
/// of being swallowed.
pub fn write_atomic(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = temp_path_for(path);

    if std::fs::write(&temp, content).is_err() {
        // Could not even stage it (read-only folder, disk full): write in place and let the error
        // reach the caller.
        return std::fs::write(path, content);
    }

    let mut attempt = 0;
    loop {
        match std::fs::rename(&temp, path) {
            Ok(()) => return Ok(()),
            Err(error) if attempt < 3 && retryable(&error) => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(40 * attempt as u64));
            }
            Err(error) => {
                // Out of retries: no debris left behind, then the old path.
                let _ = std::fs::remove_file(&temp);
                return std::fs::write(path, content).map_err(|_| error);
            }
        }
    }
}

/// Errors a second try can fix: on Windows a sync client or an antivirus holds the target open for
/// a moment and the replace fails with one of these rather than with anything meaningful.
fn retryable(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound | io::ErrorKind::WouldBlock)
}

/// Creates `dir/<name>.md` with a title line, like the upstream `file:create`. Refuses a name that
/// is empty, holds a path separator, or already exists - a note is never silently overwritten.
pub fn create_note(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let name = name.trim();
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "a note name is a single file name"));
    }
    let path = dir.join(format!("{name}.md"));
    if path.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", path.display())));
    }
    std::fs::create_dir_all(dir)?;
    write_atomic(&path, &format!("# {name}\n\n"))?;
    Ok(path)
}

/// Renames a note inside its own folder. A name without an extension keeps the old one, so
/// `renomear("Nota", "Outra")` writes `Outra.md` and never `Outra`.
pub fn rename_note(path: &Path, new_name: &str) -> io::Result<PathBuf> {
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name.contains(['/', '\\']) || new_name.contains("..") {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "a note name is a single file name"));
    }
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("md");
    let final_name = if Path::new(new_name).extension().is_some() { new_name.to_string() } else { format!("{new_name}.{extension}") };
    let target = dir.join(final_name);
    if target != path && target.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", target.display())));
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory of this test, removed when the guard drops.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("localnotebook-test-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch dir");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_temp_file_sits_beside_the_note_and_starts_with_a_dot() {
        let temp = temp_path_for(Path::new("/v/Pasta/Nota.md"));
        assert_eq!(temp, Path::new("/v/Pasta/.Nota.md.saving"));
        assert!(temp.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.')));
    }

    #[test]
    fn a_save_replaces_the_note_and_leaves_no_temp_behind() {
        let scratch = Scratch::new("save");
        let path = scratch.0.join("Nota.md");
        write_atomic(&path, "primeiro").expect("first save");
        write_atomic(&path, "segundo").expect("second save");
        assert_eq!(read_note(&path).expect("read"), "segundo");
        assert!(!temp_path_for(&path).exists(), "the staging file stayed behind");
    }

    #[test]
    fn a_save_creates_the_folder_it_needs() {
        let scratch = Scratch::new("mkdir");
        let path = scratch.0.join("Pasta").join("Nova").join("Nota.md");
        write_atomic(&path, "x").expect("save");
        assert!(path.exists());
    }

    #[test]
    fn a_new_note_starts_with_its_title_and_is_never_overwritten() {
        let scratch = Scratch::new("create");
        let path = create_note(&scratch.0, "Receita").expect("create");
        assert_eq!(read_note(&path).expect("read"), "# Receita\n\n");
        let again = create_note(&scratch.0, "Receita");
        assert_eq!(again.expect_err("must refuse").kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn a_note_name_cannot_escape_its_folder() {
        let scratch = Scratch::new("escape");
        for name in ["../fora", "pasta/nota", "  ", ".."] {
            assert!(create_note(&scratch.0, name).is_err(), "{name} was accepted");
        }
    }

    #[test]
    fn a_rename_keeps_the_extension() {
        let scratch = Scratch::new("rename");
        let path = create_note(&scratch.0, "Antiga").expect("create");
        let renamed = rename_note(&path, "Nova").expect("rename");
        assert_eq!(renamed.file_name().and_then(|n| n.to_str()), Some("Nova.md"));
        assert!(!path.exists());
    }

    #[test]
    fn a_rename_does_not_overwrite_a_note_that_exists() {
        let scratch = Scratch::new("rename-clash");
        let first = create_note(&scratch.0, "Antiga").expect("create");
        let _second = create_note(&scratch.0, "Ocupada").expect("create");
        assert_eq!(rename_note(&first, "Ocupada").expect_err("must refuse").kind(), io::ErrorKind::AlreadyExists);
    }
}
