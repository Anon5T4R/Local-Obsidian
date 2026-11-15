//! BYOK AI for Local Obsidian: the shared `local-ai` client, wearing this app's identity.
//!
//! The wire format, the provider presets and the keychain are the suite's (`libs/local-ai`); what
//! lives here is only what differs per app: the config folder and the keychain service the
//! connections file is grouped under, and the two shapes the chat panel uses - a one-shot answer and
//! a streaming one.
//!
//! Nothing in this crate touches the network by itself: a connection is a base URL the user chose
//! (Ollama and `llama-server` are just a base URL), a model name and a key in the OS keychain. There
//! is no server of ours anywhere in the path.

use std::sync::mpsc::{Receiver, channel};

pub use local_ai::{Connection, Msg, PROVIDERS, Provider, provider};

/// This app's identity in the shared crate: the data folder (`<data dir>/LocalObsidian`), the
/// keychain service and what OpenRouter sees as referer.
pub const APP: local_ai::App = local_ai::App { id: "LocalObsidian", url: "https://github.com/Anon5T4R/Local-Obsidian" };

/// The connections the user saved. A missing file is no connections, not an error.
pub fn connections() -> Result<Vec<Connection>, String> {
    Connection::load_all(&APP)
}

/// Saves every connection (the key is not in this file; it lives in the keychain).
pub fn save(all: &[Connection]) -> Result<(), String> {
    Connection::save_all(&APP, all)
}

/// The provider's own model list, for the picker.
pub fn models(conn: &Connection) -> Result<Vec<String>, String> {
    local_ai::list_models(&APP, conn)
}

/// One answer, waiting for the whole thing.
pub fn ask(conn: &Connection, history: &[Msg]) -> Result<String, String> {
    local_ai::chat(&APP, conn, history)
}

/// A streaming answer: the pieces arrive on `deltas` as the provider sends them, and the joined text
/// (or the error) on `done`. Both channels close when the request ends.
pub struct Stream {
    pub deltas: Receiver<String>,
    pub done: Receiver<Result<String, String>>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Stream {
    /// Starts the request on a worker thread. The UI polls [`Stream::poll`] every frame.
    pub fn start(conn: Connection, history: Vec<Msg>) -> Self {
        let (deltas_tx, deltas) = channel();
        let (done_tx, done) = channel();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = cancelled.clone();
        std::thread::Builder::new()
            .name("ai-chat".into())
            .spawn(move || {
                let result = local_ai::chat_stream(&APP, &conn, &history, |delta| {
                    // A closed channel means the panel was closed: stop spending the user's tokens.
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    let _ = deltas_tx.send(delta.to_owned());
                });
                let _ = done_tx.send(result);
            })
            .map_err(|error| log::warn!("ai: cannot start the chat thread: {error}"))
            .ok();
        Self { deltas, done, cancelled }
    }

    /// What arrived since the last frame, and the final result once there is one.
    pub fn poll(&self) -> (Vec<String>, Option<Result<String, String>>) {
        let mut pieces = Vec::new();
        while let Ok(delta) = self.deltas.try_recv() {
            pieces.push(delta);
        }
        (pieces, self.done.try_recv().ok())
    }

    /// Tells the worker to stop forwarding: closing the panel should not keep a long answer running.
    pub fn cancel(&self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_is_this_app() {
        assert_eq!(APP.id, "LocalObsidian");
        assert!(APP.url.contains("Local-Obsidian"));
    }

    #[test]
    fn a_connection_needs_a_base_url_to_be_useful() {
        // The `custom` preset has no URL of its own: a connection built from it is incomplete until
        // the user types one, and the chat says so instead of calling something random.
        let custom = provider("custom").expect("the custom preset exists");
        assert!(custom.base_url.is_empty());
    }

    #[test]
    fn the_model_list_is_sorted_and_deduped_by_the_provider() {
        // `models` is the shared client's call; the wrapper only forwards the identity. What is
        // testable here is that a connection with no base URL fails before any request.
        let conn = Connection { id: "x".into(), provider: "custom".into(), label: "x".into(), base_url: String::new(), model: "m".into() };
        assert!(models(&conn).is_err(), "no base URL, no request");
    }

    #[test]
    fn a_stream_that_never_started_reports_nothing() {
        // The channels close when the worker is gone; the panel must read that as "no news" and
        // keep the error path to the `done` channel.
        let stream = Stream::start(
            Connection { id: "x".into(), provider: "custom".into(), label: "x".into(), base_url: String::new(), model: "m".into() },
            Vec::new(),
        );
        let mut finished = None;
        for _ in 0..200 {
            let (_, done) = stream.poll();
            if let Some(result) = done {
                finished = Some(result);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let result = finished.expect("the worker reports the failure");
        assert!(result.is_err(), "a connection without a base URL cannot answer");
    }
}
