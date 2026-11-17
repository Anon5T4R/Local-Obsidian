//! The native folder picker, off the UI thread.
//!
//! A synchronous `rfd::FileDialog` on the UI thread re-enters winit's event handler on Windows and
//! can abort the app, so the mold is local-pdf's: `rfd::AsyncFileDialog` driven by `pollster` on a
//! worker thread, one picker at a time, the answer drained on the next frame (like
//! `updater.tick`). rfd 0.17 keeps its default backends: the native dialog on Windows, the XDG
//! portal through zbus (pure Rust) on Linux, a Zenity fallback when there is no portal.
//!
//! A vault is a folder, so the folder picker is the only one the app needs. Opening a picker never
//! blocks the frame, and a cancelled dialog is silence.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::tl;

/// What a finished picker gives back: a folder to open, or a document to import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    Folder(PathBuf),
    Document(PathBuf),
    /// An Anki text export (`.txt`, `.tsv`, `.csv`).
    Anki(PathBuf),
    /// Where the user wants a file written (an HTML page).
    Save(PathBuf),
    /// Where the user wants the note written as a PDF.
    SavePdf(PathBuf),
}

/// The picker in flight, plus the picks waiting for the frame that consumes them.
#[derive(Default)]
pub struct Pickers {
    showing: Arc<AtomicBool>,
    done: Arc<Mutex<Vec<Pick>>>,
}

impl Pickers {
    /// Whether a picker is open (or its answer still waits for the next frame).
    pub fn busy(&self) -> bool {
        self.showing.load(Ordering::SeqCst)
    }

    /// Asks for one folder. `title` is the dialog's title (already translated), `start` the
    /// directory it opens in. The answer arrives in [`Self::take`] on a later frame; `false` = a
    /// picker is already open (or the worker thread could not start).
    pub fn ask_folder(&self, ctx: &egui::Context, title: &str, start: Option<&str>) -> bool {
        let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
        if let Some(dir) = start.filter(|dir| !dir.is_empty()) {
            dialog = dialog.set_directory(dir);
        }
        self.run(ctx, move || pollster::block_on(dialog.pick_folder()).map(|handle| Pick::Folder(handle.path().to_path_buf())))
    }

    /// Asks for one document to import. Same contract as [`Self::ask_folder`].
    pub fn ask_document(&self, ctx: &egui::Context, title: &str, start: Option<&str>) -> bool {
        let mut dialog = rfd::AsyncFileDialog::new().set_title(title).add_filter(tl!("Word or OpenDocument text"), &["docx", "odt"]);
        if let Some(dir) = start.filter(|dir| !dir.is_empty()) {
            dialog = dialog.set_directory(dir);
        }
        self.run(ctx, move || pollster::block_on(dialog.pick_file()).map(|handle| Pick::Document(handle.path().to_path_buf())))
    }

    /// Asks for an Anki text export to import. Same contract as [`Self::ask_folder`].
    pub fn ask_anki(&self, ctx: &egui::Context, title: &str, start: Option<&str>) -> bool {
        let mut dialog = rfd::AsyncFileDialog::new().set_title(title).add_filter(tl!("Anki text export"), &["txt", "tsv", "csv"]);
        if let Some(dir) = start.filter(|dir| !dir.is_empty()) {
            dialog = dialog.set_directory(dir);
        }
        self.run(ctx, move || pollster::block_on(dialog.pick_file()).map(|handle| Pick::Anki(handle.path().to_path_buf())))
    }

    /// Asks where to write a file, suggesting `file_name`. Same contract as [`Self::ask_folder`].
    pub fn ask_save(&self, ctx: &egui::Context, title: &str, file_name: &str) -> bool {
        let dialog = rfd::AsyncFileDialog::new().set_title(title).set_file_name(file_name).add_filter(tl!("HTML page"), &["html"]);
        self.run(ctx, move || pollster::block_on(dialog.save_file()).map(|handle| Pick::Save(handle.path().to_path_buf())))
    }

    /// Asks where to write the note as a PDF, suggesting `file_name`. Same contract as [`Self::ask_folder`].
    pub fn ask_pdf(&self, ctx: &egui::Context, title: &str, file_name: &str) -> bool {
        let dialog = rfd::AsyncFileDialog::new().set_title(title).set_file_name(file_name).add_filter(tl!("PDF document"), &["pdf"]);
        self.run(ctx, move || pollster::block_on(dialog.save_file()).map(|handle| Pick::SavePdf(handle.path().to_path_buf())))
    }

    /// Runs `dialog` on a worker thread; its result is queued, and the frame that asked is woken up.
    fn run(&self, ctx: &egui::Context, dialog: impl FnOnce() -> Option<Pick> + Send + 'static) -> bool {
        if self.busy() {
            return false;
        }
        self.showing.store(true, Ordering::SeqCst);
        let showing = self.showing.clone();
        let done = self.done.clone();
        let ctx = ctx.clone(); // an Arc inside: cheap; the thread needs 'static
        std::thread::Builder::new()
            .name("file-picker".into())
            .spawn(move || {
                if let Some(pick) = dialog() {
                    done.lock().unwrap_or_else(PoisonError::into_inner).push(pick);
                }
                showing.store(false, Ordering::SeqCst);
                ctx.request_repaint(); // wakes the frame that consumes the answer
            })
            .is_ok()
    }

    /// The picks made so far; the frame that asks clears the queue.
    pub fn take(&self) -> Vec<Pick> {
        std::mem::take(&mut *self.done.lock().unwrap_or_else(PoisonError::into_inner))
    }
}
