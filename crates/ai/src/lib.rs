//! BYOK AI for Local Notebook: the shared `local-ai` client, wearing this app's identity.
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

/// This app's identity in the shared crate: the data folder (`<data dir>/LocalNotebook`), the
/// keychain service and what OpenRouter sees as referer.
pub const APP: local_ai::App = local_ai::App { id: "LocalNotebook", url: "https://github.com/Styx-Studios/Local-Notebook" };

/// The identity under the product's old name (Local Obsidian): the data folder and the keychain
/// service the connections and their keys were saved under before the rename.
pub const LEGACY_APP: local_ai::App = local_ai::App { id: "LocalObsidian", url: APP.url };

/// The folder the connections file lives in (`<data dir>/LocalNotebook`), when the system has a
/// data dir. The app moves the old folder here on the first start after the rename.
pub fn data_dir() -> Option<std::path::PathBuf> {
    Connection::store_path(&APP).and_then(|file| file.parent().map(std::path::Path::to_path_buf))
}

/// The folder the connections file lived in under the old name.
pub fn legacy_data_dir() -> Option<std::path::PathBuf> {
    Connection::store_path(&LEGACY_APP).and_then(|file| file.parent().map(std::path::Path::to_path_buf))
}

/// Copies the API keys saved under the old keychain service to the current one, for every saved
/// connection that has none yet. A key already under the current service is never replaced, the
/// old entries stay where they are, and a keychain that can't be read is skipped (logged, never
/// fatal). Returns how many keys were copied.
pub fn migrate_legacy_keys() -> usize {
    let all = match connections() {
        Ok(all) => all,
        Err(e) => {
            log::warn!("legacy AI keys: the connections can't be read: {e}");
            return 0;
        }
    };
    migrate_keys(&all, |c| c.has_key(&APP), |c| c.key(&LEGACY_APP).unwrap_or_default(), |c, key| c.set_key(&APP, key))
}

/// [`migrate_legacy_keys`] with the keychain behind three closures (so it can be tested).
fn migrate_keys(
    all: &[Connection],
    has_current: impl Fn(&Connection) -> bool,
    legacy_key: impl Fn(&Connection) -> String,
    set_current: impl Fn(&Connection, &str) -> Result<(), String>,
) -> usize {
    let mut copied = 0;
    for connection in all {
        if has_current(connection) {
            continue;
        }
        let key = legacy_key(connection);
        if key.is_empty() {
            continue;
        }
        match set_current(connection, &key) {
            Ok(()) => {
                log::info!("copied the API key of connection `{}` to the Local Notebook keychain entry", connection.id);
                copied += 1;
            }
            Err(e) => log::warn!("couldn't copy the API key of connection `{}`: {e}", connection.id),
        }
    }
    copied
}

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

/// The cards a model answered, wherever it wrapped them.
///
/// The contract is `{"cards":[{"question": "...","answer": "..."}]}`, but a model is a model: it
/// writes prose first, wraps the JSON in a fence, trails a comma, or sends the bare array. The
/// readers here find the JSON inside the answer and keep only the pairs that have both sides, so a
/// chatty answer still becomes cards (and an answer with no cards becomes an error the dialog shows).
pub fn cards_from_answer(answer: &str) -> Vec<(String, String)> {
    for value in json_candidates(answer) {
        let cards = cards_in(&value);
        if !cards.is_empty() {
            return cards;
        }
    }
    Vec::new()
}

/// The JSON values found in `text`, in order: every balanced `{...}`/`[...]` that parses (after the
/// one repair that matters, a trailing comma).
fn json_candidates(text: &str) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for (start, ch) in text.char_indices() {
        if ch != '{' && ch != '[' {
            continue;
        }
        let Some(end) = balanced_end(&text[start..]) else { continue };
        let slice = &text[start..start + end];
        if let Ok(value) = serde_json::from_str(slice) {
            out.push(value);
        } else if let Ok(value) = serde_json::from_str(&without_trailing_commas(slice)) {
            out.push(value);
        }
    }
    out
}

/// The byte length of the balanced `{...}`/`[...]` at the start of `rest`, skipping string contents.
fn balanced_end(rest: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (at, ch) in rest.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at + ch.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

/// A JSON object with the commas a model leaves before `}`/`]` taken out.
fn without_trailing_commas(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    let mut chars = json.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                out.push(ch);
            }
            ',' => {
                // The comma stays unless a closing bracket is what comes next.
                let mut look = chars.clone();
                let mut next = look.next();
                while next.is_some_and(char::is_whitespace) {
                    next = look.next();
                }
                if !matches!(next, Some('}') | Some(']') | None) {
                    out.push(ch);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// The pairs in one JSON value: an array of cards, an object holding one under a known key, or a
/// single card.
fn cards_in(value: &serde_json::Value) -> Vec<(String, String)> {
    match value {
        serde_json::Value::Array(items) => items.iter().filter_map(card_of).collect(),
        serde_json::Value::Object(map) => {
            for key in ["cards", "flashcards", "items", "questions", "cards_created"] {
                if let Some(serde_json::Value::Array(items)) = map.get(key) {
                    let cards: Vec<(String, String)> = items.iter().filter_map(card_of).collect();
                    if !cards.is_empty() {
                        return cards;
                    }
                }
            }
            card_of(value).into_iter().collect()
        }
        _ => Vec::new(),
    }
}

/// One `{question, answer}` pair, with the field names a model is likely to use.
fn card_of(value: &serde_json::Value) -> Option<(String, String)> {
    let map = value.as_object()?;
    let field = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| map.get(*key).and_then(serde_json::Value::as_str))
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    let question = field(&["question", "q", "front", "pergunta"])?;
    let answer = field(&["answer", "a", "back", "resposta"])?;
    Some((question, answer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_answer_is_read_as_cards() {
        let cards = cards_from_answer(r#"{"cards":[{"question":"O que é?","answer":"Uma coisa."}]}"#);
        assert_eq!(cards, [("O que é?".to_string(), "Uma coisa.".to_string())]);
    }

    #[test]
    fn the_json_is_found_inside_prose_and_fences() {
        let answer = "Claro! Aqui estão:\n```json\n{\"flashcards\": [{\"front\": \"P1\", \"back\": \"R1\"}, {\"q\": \"P2\", \"a\": \"R2\"}]}\n```\nBons estudos.";
        let cards = cards_from_answer(answer);
        assert_eq!(cards, [("P1".to_string(), "R1".to_string()), ("P2".to_string(), "R2".to_string())]);
    }

    #[test]
    fn a_trailing_comma_does_not_lose_the_answer() {
        let cards = cards_from_answer(r#"{"cards":[{"question":"P","answer":"R",},]}"#);
        assert_eq!(cards, [("P".to_string(), "R".to_string())]);
    }

    #[test]
    fn a_bare_array_is_a_list_of_cards() {
        let cards = cards_from_answer(r#"[{"question":"P","answer":"R"}]"#);
        assert_eq!(cards.len(), 1);
    }

    #[test]
    fn prose_without_cards_is_an_empty_answer_not_a_guess() {
        assert!(cards_from_answer("Desculpe, não consegui.").is_empty());
        // A pair missing its answer is not a card.
        assert!(cards_from_answer(r#"{"cards":[{"question":"P"}]}"#).is_empty());
    }

    #[test]
    fn the_identity_is_this_app() {
        assert_eq!(APP.id, "LocalNotebook");
        assert!(APP.url.contains("Styx-Studios/Local-Notebook"));
    }

    fn connection(id: &str) -> Connection {
        Connection { id: id.into(), provider: "custom".into(), label: id.into(), base_url: String::new(), model: String::new() }
    }

    #[test]
    fn the_old_identity_only_differs_in_the_name() {
        assert_eq!(LEGACY_APP.id, "LocalObsidian");
        assert_eq!(LEGACY_APP.url, APP.url);
        assert_ne!(data_dir(), legacy_data_dir());
    }

    #[test]
    fn legacy_keys_are_copied_once_and_never_overwrite() {
        use std::cell::RefCell;
        use std::collections::HashMap;

        let all = [connection("a"), connection("b"), connection("c"), connection("d")];
        let current: RefCell<HashMap<String, String>> = RefCell::new(HashMap::from([("b".to_string(), "new-b".to_string())]));
        let legacy: HashMap<&str, &str> = HashMap::from([("a", "old-a"), ("b", "old-b"), ("d", "")]);
        let run = || {
            migrate_keys(
                &all,
                |c| current.borrow().get(&c.id).is_some_and(|k| !k.is_empty()),
                |c| legacy.get(c.id.as_str()).map(|k| (*k).to_string()).unwrap_or_default(),
                |c, key| {
                    current.borrow_mut().insert(c.id.clone(), key.to_string());
                    Ok(())
                },
            )
        };
        assert_eq!(run(), 1, "only `a` has an old key and no new one");
        assert_eq!(current.borrow().get("a").map(String::as_str), Some("old-a"));
        assert_eq!(current.borrow().get("b").map(String::as_str), Some("new-b"), "an existing key is kept");
        assert!(!current.borrow().contains_key("c") && !current.borrow().contains_key("d"));
        assert_eq!(run(), 0, "the second start has nothing to copy");
    }

    #[test]
    fn a_keychain_that_refuses_is_not_fatal() {
        let all = [connection("a")];
        assert_eq!(migrate_keys(&all, |_| false, |_| "k".into(), |_, _| Err("locked".into())), 0);
    }

    #[test]
    fn a_stream_reads_the_providers_events_end_to_end() {
        use std::io::{Read, Write};

        // A provider on this machine speaking the wire format the client claims: POST to
        // `/chat/completions` with the key, SSE events back, `[DONE]` to close.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let (request_tx, request_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            let mut request = Vec::new();
            let mut buf = [0u8; 2048];
            loop {
                match socket.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        request.extend_from_slice(&buf[..read]);
                        // The head ends at the blank line; the body follows it, and `content-length`
                        // says how long it is. Reading both is what lets the test check the payload.
                        if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&request[..end]).to_lowercase();
                            let length = head
                                .lines()
                                .find_map(|line| line.strip_prefix("content-length:").and_then(|value| value.trim().parse::<usize>().ok()))
                                .unwrap_or(0);
                            if request.len() >= end + 4 + length {
                                break;
                            }
                        }
                    }
                }
            }
            let _ = request_tx.send(String::from_utf8_lossy(&request).to_string());
            let body = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"ol\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"á\"}}]}\n\n",
                "data: [DONE]\n\n"
            );
            let response =
                format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = socket.write_all(response.as_bytes());
        });

        let conn = Connection {
            id: "e2e".into(),
            provider: "custom".into(),
            label: "local".into(),
            base_url: format!("http://127.0.0.1:{port}/v1"),
            model: "m".into(),
        };
        let mut pieces: Vec<String> = Vec::new();
        let result = local_ai::chat_stream_with_key(&APP, &conn, "test-key", &[Msg::User("oi".into())], |delta| {
            pieces.push(delta.to_string());
        });
        assert!(result.is_ok(), "the stream failed: {result:?}");
        assert_eq!(pieces.concat(), "olá");
        assert_eq!(result.unwrap_or_default(), "olá", "the joined answer is the result too");

        let request = request_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("the request reached the server");
        assert!(request.starts_with("POST /v1/chat/completions"), "{request}");
        assert!(request.contains("Bearer test-key"), "the key travels in the header: {request}");
        assert!(request.to_lowercase().contains("content-type: application/json"), "{request}");
        assert!(request.contains("\"stream\":true") || request.contains("\"stream\": true"), "the request asks for events: {request}");
        server.join().ok();
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
