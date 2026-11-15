//! SM-2, the classic Anki algorithm, plus the file the schedule lives in.
//!
//! Port of the upstream `srs.ts`. The maths is deliberately identical to the desktop's and the
//! Android app's: the same `.openobsidian/srs.json` is read by either, so a card reviewed on the
//! phone is not asked again on the computer that evening. A bug here silently ruins months of
//! study, which is why nothing in this file touches the filesystem except [`load`] and [`save`].

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{Duration, Local, NaiveDate};
use serde_json::Value;

/// The four answers a review session offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    Again,
    Hard,
    Good,
    Easy,
}

/// One scheduled card, exactly as `srs.json` stores it.
#[derive(Debug, Clone, PartialEq)]
pub struct CardState {
    /// The note the card lives in, relative to the vault root.
    pub file: String,
    /// The question, kept so the review panel need not re-read the note to show it.
    pub q: String,
    pub ease: f64,
    /// Days until the next review (fractional right after an ease multiplication, like upstream).
    pub interval: f64,
    pub reps: i64,
    /// ISO date (`YYYY-MM-DD`).
    pub due: String,
    pub lapses: i64,
    pub suspended: bool,
}

/// The whole schedule: what `.openobsidian/srs.json` holds.
#[derive(Debug, Clone, PartialEq)]
pub struct SrsFile {
    pub version: u32,
    /// Card id → state.
    pub cards: BTreeMap<String, CardState>,
}

impl Default for SrsFile {
    fn default() -> Self {
        Self { version: 1, cards: BTreeMap::new() }
    }
}

/// Nothing is scheduled before this ease.
const MIN_EASE: f64 = 1.3;

/// The local date, `YYYY-MM-DD`. Local on purpose: a card due today is due today where the user is.
pub fn today_iso(now: chrono::DateTime<Local>) -> String {
    now.date_naive().format("%Y-%m-%d").to_string()
}

/// Today's date, from the system clock (what the panels and the sync use).
pub fn today() -> String {
    today_iso(Local::now())
}

/// Today plus `days`, from the system clock.
pub fn date_in(days: i64) -> String {
    add_days(&today(), days as f64)
}

/// `iso` plus `days`, rounded the way the other apps round it.
pub fn add_days(iso: &str, days: f64) -> String {
    let Ok(base) = NaiveDate::parse_from_str(iso, "%Y-%m-%d") else { return iso.to_string() };
    let shifted = base + Duration::days(days.round() as i64);
    shifted.format("%Y-%m-%d").to_string()
}

/// A new card: due today, because writing a card means wanting to learn it.
pub fn new_card(file: &str, q: &str, today: &str) -> CardState {
    CardState { file: file.to_string(), q: q.to_string(), ease: 2.5, interval: 0.0, reps: 0, due: today.to_string(), lapses: 0, suspended: false }
}

/// SM-2. Returns a new state - never mutates, so a failed write cannot leave the schedule
/// half-updated.
pub fn grade(card: &CardState, grade: Grade, today: &str) -> CardState {
    if grade == Grade::Again {
        return CardState {
            reps: 0,
            interval: 0.0,
            ease: (card.ease - 0.2).max(MIN_EASE),
            lapses: card.lapses + 1,
            // Seen again in this same session.
            due: today.to_string(),
            ..card.clone()
        };
    }

    let interval = if card.reps == 0 {
        if grade == Grade::Easy { 4.0 } else { 1.0 }
    } else if card.reps == 1 {
        if grade == Grade::Easy { 8.0 } else { 6.0 }
    } else {
        let factor = match grade {
            Grade::Hard => 1.2,
            Grade::Easy => card.ease * 1.3,
            _ => card.ease,
        };
        (card.interval * factor).max(1.0)
    };

    let ease = match grade {
        Grade::Hard => (card.ease - 0.15).max(MIN_EASE),
        Grade::Easy => card.ease + 0.15,
        _ => card.ease,
    };

    CardState { reps: card.reps + 1, interval, ease, due: add_days(today, interval), ..card.clone() }
}

/// Whether the schedule asks for this card today.
pub fn is_due(card: &CardState, today: &str) -> bool {
    !card.suspended && card.due.as_str() <= today
}

/// Cards to review, oldest due date first so the backlog drains in order.
pub fn due_cards(file: &SrsFile, today: &str) -> Vec<(String, CardState)> {
    let mut due: Vec<(String, CardState)> =
        file.cards.iter().filter(|(_, card)| is_due(card, today)).map(|(id, card)| (id.clone(), card.clone())).collect();
    due.sort_by(|a, b| a.1.due.cmp(&b.1.due).then_with(|| a.1.q.cmp(&b.1.q)));
    due
}

/// How the vault's cards are doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SrsStats {
    pub total: usize,
    pub due: usize,
    pub suspended: usize,
    /// Cards never reviewed.
    pub fresh: usize,
}

pub fn stats(file: &SrsFile, today: &str) -> SrsStats {
    let cards: Vec<&CardState> = file.cards.values().collect();
    SrsStats {
        total: cards.len(),
        due: cards.iter().filter(|card| is_due(card, today)).count(),
        suspended: cards.iter().filter(|card| card.suspended).count(),
        fresh: cards.iter().filter(|card| card.reps == 0).count(),
    }
}

/// The statistics panel's numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct SrsReport {
    pub stats: SrsStats,
    /// Reviewed at least once and never failed since.
    pub learned: usize,
    /// Share of reviewed cards that never lapsed, 0-1.
    pub retention: f64,
    pub average_ease: f64,
    /// How many come due on each of the next 14 days.
    pub forecast: Vec<(String, usize)>,
    /// The notes holding the most cards.
    pub top_files: Vec<(String, usize)>,
}

pub fn report(file: &SrsFile, today: &str) -> SrsReport {
    let cards: Vec<&CardState> = file.cards.values().collect();
    let reviewed: Vec<&&CardState> = cards.iter().filter(|card| card.reps > 0).collect();
    let clean: Vec<&&&CardState> = reviewed.iter().filter(|card| card.lapses == 0).collect();

    let mut forecast = Vec::with_capacity(14);
    for offset in 0..14 {
        let date = add_days(today, offset as f64);
        // Day 0 carries the backlog: everything overdue is waiting today.
        let count =
            cards.iter().filter(|card| !card.suspended && if offset == 0 { card.due.as_str() <= date.as_str() } else { card.due == date }).count();
        forecast.push((date, count));
    }

    let mut by_file: BTreeMap<&str, usize> = BTreeMap::new();
    for card in &cards {
        *by_file.entry(card.file.as_str()).or_default() += 1;
    }
    let mut top_files: Vec<(String, usize)> = by_file.into_iter().map(|(file, count)| (file.to_string(), count)).collect();
    top_files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    top_files.truncate(8);

    SrsReport {
        stats: stats(file, today),
        learned: clean.len(),
        retention: if reviewed.is_empty() { 0.0 } else { clean.len() as f64 / reviewed.len() as f64 },
        average_ease: if cards.is_empty() { 0.0 } else { cards.iter().map(|card| card.ease).sum::<f64>() / cards.len() as f64 },
        forecast,
        top_files,
    }
}

/// Reconciles the cards a note declares with what the schedule knows.
///
/// A card keeps its state while its id does not change - and the id is a hash of the *question*, so
/// editing the answer never resets the schedule. A card that disappeared from the note is pruned,
/// but only within the file being synced.
pub fn sync_file(srs: &SrsFile, file: &str, found: &[(String, String)], today: &str) -> (SrsFile, usize, usize) {
    let mut cards = srs.cards.clone();
    let seen: Vec<&str> = found.iter().map(|(id, _)| id.as_str()).collect();
    let mut added = 0;
    let mut removed = 0;

    for (id, question) in found {
        match cards.get_mut(id) {
            Some(existing) => {
                if existing.file != file || existing.q != *question {
                    existing.file = file.to_string();
                    existing.q = question.clone();
                }
            }
            None => {
                cards.insert(id.clone(), new_card(file, question, today));
                added += 1;
            }
        }
    }

    let stale: Vec<String> = cards.iter().filter(|(id, card)| card.file == file && !seen.contains(&id.as_str())).map(|(id, _)| id.clone()).collect();
    for id in stale {
        cards.remove(&id);
        removed += 1;
    }
    (SrsFile { version: 1, cards }, added, removed)
}

/// [`sync_file`] for the whole vault in one pass, so a big vault is two walks instead of one per
/// note. A note missing from `found_by_file` was not readable this pass: its schedule stays as is.
pub fn sync_vault(srs: &SrsFile, found_by_file: &BTreeMap<String, Vec<(String, String)>>, today: &str) -> SrsFile {
    let mut cards = srs.cards.clone();
    // Notes that were scanned: their cards must be exactly the ones found there.
    for (file, found) in found_by_file {
        let ids: Vec<&str> = found.iter().map(|(id, _)| id.as_str()).collect();
        cards.retain(|id, card| card.file != *file || ids.contains(&id.as_str()));
    }
    for (file, found) in found_by_file {
        for (id, question) in found {
            match cards.get_mut(id) {
                Some(existing) => {
                    existing.file = file.clone();
                    existing.q = question.clone();
                }
                None => {
                    cards.insert(id.clone(), new_card(file, question, today));
                }
            }
        }
    }
    SrsFile { version: 1, cards }
}

// ---- the file ------------------------------------------------------------------------------

/// Reads the schedule. A missing or malformed file is an empty schedule, never a crash; cards
/// whose `file` is an absolute path (a bug of an older version) are dropped and counted.
pub fn load(path: &Path) -> (SrsFile, usize) {
    let Ok(text) = std::fs::read_to_string(path) else { return (SrsFile::default(), 0) };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        log::warn!("srs: {} is not valid JSON; starting an empty schedule", path.display());
        return (SrsFile::default(), 0);
    };
    let mut file = SrsFile::default();
    if let Some(version) = value.get("version").and_then(Value::as_u64) {
        file.version = version as u32;
    }
    let Some(cards) = value.get("cards").and_then(Value::as_object) else { return (file, 0) };

    for (id, card) in cards {
        let Some(object) = card.as_object() else { continue };
        let text_field = |key: &str| object.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        file.cards.insert(
            id.clone(),
            CardState {
                file: text_field("file"),
                q: text_field("q"),
                ease: object.get("ease").and_then(Value::as_f64).unwrap_or(2.5),
                interval: object.get("interval").and_then(Value::as_f64).unwrap_or(0.0),
                reps: object.get("reps").and_then(Value::as_i64).unwrap_or(0),
                due: {
                    let due = text_field("due");
                    if due.is_empty() { today_iso(Local::now()) } else { due }
                },
                lapses: object.get("lapses").and_then(Value::as_i64).unwrap_or(0),
                suspended: object.get("suspended").and_then(Value::as_bool).unwrap_or(false),
            },
        );
    }

    // Cards from before the relative-path fix carry an absolute path: they belong to no note.
    let absolute: Vec<String> = file.cards.iter().filter(|(_, card)| looks_absolute(&card.file)).map(|(id, _)| id.clone()).collect();
    let dropped = absolute.len();
    for id in absolute {
        file.cards.remove(&id);
    }
    (file, dropped)
}

fn looks_absolute(file: &str) -> bool {
    file.starts_with('/') || file.starts_with('\\') || file.get(1..2) == Some(":")
}

/// Writes the schedule.
///
/// The bytes are built here, not by a serializer, so the field order is the one the desktop and the
/// Android app write (`file`, `q`, `ease`, `interval`, `reps`, `due`, `lapses`, `suspended?`):
/// `srs.json` lives in a folder people sync, and a different order on every save would rewrite the
/// whole file in every diff.
pub fn save(path: &Path, file: &SrsFile) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = String::with_capacity(64 + file.cards.len() * 160);
    out.push_str("{\n  \"version\": ");
    out.push_str(&file.version.to_string());
    out.push_str(",\n  \"cards\": {");
    for (index, (id, card)) in file.cards.iter().enumerate() {
        out.push_str(if index == 0 { "\n" } else { ",\n" });
        out.push_str("    ");
        out.push_str(&json_string(id));
        out.push_str(": {\n      \"file\": ");
        out.push_str(&json_string(&card.file));
        out.push_str(",\n      \"q\": ");
        out.push_str(&json_string(&card.q));
        out.push_str(",\n      \"ease\": ");
        out.push_str(&number(card.ease));
        out.push_str(",\n      \"interval\": ");
        out.push_str(&number(card.interval));
        out.push_str(",\n      \"reps\": ");
        out.push_str(&card.reps.to_string());
        out.push_str(",\n      \"due\": ");
        out.push_str(&json_string(&card.due));
        out.push_str(",\n      \"lapses\": ");
        out.push_str(&card.lapses.to_string());
        if card.suspended {
            out.push_str(",\n      \"suspended\": true");
        }
        out.push_str("\n    }");
    }
    if file.cards.is_empty() {
        out.push_str("}\n}\n");
    } else {
        out.push_str("\n  }\n}\n");
    }
    write_atomic(path, &out)
}

/// A JSON string, escaped by the serializer (a question may hold quotes and newlines).
fn json_string(value: &str) -> String {
    Value::String(value.to_string()).to_string()
}

/// Writes the file the way a note is written: a temp file beside it, then a rename, so a crash in
/// the middle cannot truncate months of scheduling.
fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let temp = path.with_extension("json.saving");
    std::fs::write(&temp, content)?;
    match std::fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = std::fs::remove_file(&temp);
            std::fs::write(path, content)
        }
    }
}

/// A number the way JavaScript prints it: an integral value keeps no decimal point only when it
/// came from an integer; here every stored number is fractional by nature, so `1.0` stays `1.0`.
fn number(value: f64) -> String {
    if value.is_finite() { serde_json::Number::from_f64(value).map_or_else(|| "0".to_string(), |number| number.to_string()) } else { "0".to_string() }
}

// ---- Anki plain-text exchange --------------------------------------------------------------

/// The cards of a note as Anki's tab-separated text.
pub fn to_anki_text(cards: &[(String, String)]) -> String {
    let clean = |text: &str| text.replace('\t', " ").replace("\r\n", "<br>").replace('\n', "<br>");
    cards.iter().map(|(q, a)| format!("{}\t{}", clean(q), clean(a))).collect::<Vec<_>>().join("\n")
}

/// One Anki card, as the text export describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnkiCard {
    pub q: String,
    pub a: String,
    pub tags: Vec<String>,
    /// A cloze note: the gaps are the answer, so an empty second field is normal.
    pub cloze: bool,
}

/// What a text export held.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AnkiImport {
    pub cards: Vec<AnkiCard>,
    /// Cards that referenced an image or audio the text export cannot carry.
    pub with_media: usize,
}

/// Anki's newer exports declare their layout on `#` lines; reading that keeps the tags from being
/// glued onto the answer.
fn read_directives(text: &str) -> (Option<usize>, Option<String>) {
    let mut tags_column = None;
    let mut separator = None;
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if !line.starts_with('#') {
            break;
        }
        let lower = line.to_lowercase();
        if let Some(rest) = lower.strip_prefix("#tags column:")
            && let Ok(column) = rest.trim().parse::<usize>()
        {
            // Anki counts from 1.
            tags_column = Some(column.saturating_sub(1));
        }
        if let Some(rest) = lower.strip_prefix("#separator:") {
            separator = Some(rest.trim().to_string());
        }
    }
    (tags_column, separator)
}

/// Whether the line references media the text export cannot carry.
fn has_media(line: &str) -> bool {
    line.to_lowercase().contains("<img") || line.to_lowercase().contains("[sound:")
}

/// Parses an Anki text export into cards, tags included.
pub fn from_anki_text(text: &str) -> AnkiImport {
    let (tags_column, separator) = read_directives(text);
    let split_on = match separator.as_deref() {
        Some("semicolon") => Some(';'),
        Some("comma") => Some(','),
        _ => None,
    };
    let mut cards = Vec::new();
    let mut with_media = 0;

    for raw in text.split('\n') {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = match split_on {
            Some(delimiter) => line.split(delimiter).collect(),
            None if line.contains('\t') => line.split('\t').collect(),
            None => line.split(';').collect(),
        };
        if parts.len() < 2 {
            continue;
        }
        if has_media(line) {
            with_media += 1;
        }

        // The tags column is metadata, never part of the answer.
        let tag_index = match tags_column {
            Some(column) if column < parts.len() => column,
            _ if parts.len() > 2 => parts.len() - 1,
            _ => usize::MAX,
        };
        let tags = if tag_index != usize::MAX && tag_index > 1 { split_tags(parts[tag_index]) } else { Vec::new() };
        let answer_parts: Vec<&str> = parts.iter().enumerate().filter(|(index, _)| *index != tag_index).skip(1).map(|(_, part)| *part).collect();

        // A cloze note carries its answer inside the sentence and its "Extra" field is usually
        // empty: requiring an answer would silently throw every one of them away.
        let cloze = is_cloze(parts[0]);
        let q = anki_field_to_markdown(parts[0], false);
        let a = anki_field_to_markdown(&answer_parts.join(" "), false);
        if !q.is_empty() && (!a.is_empty() || cloze) {
            cards.push(AnkiCard { q, a, tags, cloze });
        }
    }
    AnkiImport { cards, with_media }
}

/// Whether the field uses Anki's cloze syntax (`{{c1::term}}`).
fn is_cloze(field: &str) -> bool {
    let mut rest = field;
    while let Some(start) = rest.find("{{c") {
        let after = &rest[start + 3..];
        let digits = after.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && after[digits..].starts_with("::") {
            return true;
        }
        rest = after;
    }
    false
}

fn split_tags(raw: &str) -> Vec<String> {
    raw.split_whitespace()
        .filter(|tag| !tag.is_empty())
        // Anki nests tags with `::`; this app nests them with `/`.
        .map(|tag| tag.trim_start_matches('#').replace("::", "/"))
        .collect()
}

/// One Anki field as Markdown: `{{c1::term}}` becomes `==term==` (this app's cloze syntax) and
/// `[$]x[/$]` becomes `$x$`.
pub fn anki_field_to_markdown(field: &str, keep_media: bool) -> String {
    let mut text = replace_cloze_gaps(field);
    text = text.replace("[$ $]", "");
    // `[$$]x[/$$]` and `[$]x[/$]` are display and inline math.
    text = replace_between(&text, "[$$]", "[/$$]", |inner| format!("$${inner}$$"));
    text = replace_between(&text, "[$]", "[/$]", |inner| format!("${inner}$"));
    text = replace_sound(&text, keep_media);
    text = replace_images(&text, keep_media);
    text = text.replace("\r\n", " ").replace('\n', " ");
    text = replace_between(&text, "<br", ">", |_| " ".to_string());
    text = strip_tags(&text);
    text = text.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">");
    collapse_spaces(&text).trim().to_string()
}

/// `{{c1::term}}` / `{{c1::term::hint}}` → `==term==`.
fn replace_cloze_gaps(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut rest = field;
    while let Some(start) = rest.find("{{c") {
        out.push_str(&rest[..start]);
        let after = start + 3;
        let Some(body_start) = rest[after..].find("::").map(|offset| after + offset + 2) else {
            out.push_str(&rest[start..]);
            return out;
        };
        let Some(end) = rest[body_start..].find("}}").map(|offset| body_start + offset) else {
            out.push_str(&rest[start..]);
            return out;
        };
        let body = &rest[body_start..end];
        // A hint after the second `::` is not part of the term.
        let term = body.split("::").next().unwrap_or(body);
        out.push_str("==");
        out.push_str(term);
        out.push_str("==");
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Replaces every `open…close` span with what `wrap` returns.
fn replace_between(text: &str, open: &str, close: &str, wrap: impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let after = start + open.len();
        let Some(end) = rest[after..].find(close).map(|offset| after + offset) else {
            out.push_str(&rest[start..]);
            return out;
        };
        out.push_str(&wrap(&rest[after..end]));
        rest = &rest[end + close.len()..];
    }
    out.push_str(rest);
    out
}

/// `[sound:file.mp3]` → nothing, or a Markdown reference when the media is kept.
fn replace_sound(text: &str, keep: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.to_lowercase().find("[sound:") {
        out.push_str(&rest[..start]);
        let after = start + "[sound:".len();
        let Some(end) = rest[after..].find(']').map(|offset| after + offset) else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &rest[after..end];
        if keep {
            out.push_str(&format!(" [{name}]({name})"));
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// `<img src="x.png">` → nothing, or a Markdown reference when the media is kept.
fn replace_images(text: &str, keep: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut from = 0;
    let mut index = 0;
    while index < bytes.len() {
        // `<img`, in any case, and never across a character boundary.
        if bytes[index] == b'<' && index + 4 <= bytes.len() && text.is_char_boundary(index + 4) && text[index..index + 4].eq_ignore_ascii_case("<img")
        {
            out.push_str(&text[from..index]);
            let Some(end) = text[index..].find('>').map(|offset| index + offset + 1) else {
                out.push_str(&text[index..]);
                return out;
            };
            if keep && let Some(source) = attribute(&text[index..end], "src") {
                out.push_str(&format!(" ![]({source})"));
            }
            from = end;
            index = end;
            continue;
        }
        index += 1;
    }
    out.push_str(&text[from..]);
    out
}

/// The value of `name="…"` in a tag.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let start = lower.find(&format!("{name}="))? + name.len() + 1;
    let rest = &tag[start..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let inner = &rest[1..];
        let end = inner.find(quote)?;
        Some(inner[..end].to_string())
    } else {
        let end = rest.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

/// Everything between `<` and `>`.
fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            other if !in_tag => out.push(other),
            _ => {}
        }
    }
    out
}

/// Two or more spaces become one.
fn collapse_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for c in text.chars() {
        if c == ' ' {
            if !last_space {
                out.push(c);
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out
}

/// Anki cards → a Markdown note made of card callouts.
pub fn anki_to_markdown(title: &str, cards: &[AnkiCard]) -> String {
    let mut tags: Vec<String> = Vec::new();
    for card in cards {
        for tag in &card.tags {
            if !tags.contains(tag) {
                tags.push(tag.clone());
            }
        }
    }
    let header =
        if tags.is_empty() { String::new() } else { format!("{}\n\n", tags.iter().map(|tag| format!("#{tag}")).collect::<Vec<_>>().join(" ")) };
    let body = cards
        .iter()
        .map(|card| {
            let quote = |text: &str| text.replace('\n', "\n> ");
            if card.cloze {
                // The Extra field, when there is one, becomes the card's title.
                let head = if card.a.is_empty() { String::new() } else { format!(" {}", quote(&card.a)) };
                format!("> [!card]{head}\n> {}", quote(&card.q))
            } else {
                format!("> [!card]- {}\n> {}", card.q, quote(&card.a))
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("# {title}\n\n{header}{body}\n")
}

/// File names the deck refers to, so only those get extracted.
pub fn collect_media_refs(cards: &[AnkiCard]) -> Vec<String> {
    let mut refs = Vec::new();
    for card in cards {
        for text in [&card.q, &card.a] {
            for target in markdown_links(text) {
                // A URL, or something already pointing at a folder, is not media here.
                if !target.is_empty() && !target.contains("://") && !target.contains('/') && !refs.contains(&target) {
                    refs.push(target);
                }
            }
        }
    }
    refs
}

/// The destinations of `[text](dest)` / `![alt](dest)`, in order.
fn markdown_links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("](") {
        let after = open + 2;
        let Some(close) = rest[after..].find(')').map(|offset| after + offset) else { break };
        let target = rest[after..close].trim().to_string();
        if !target.is_empty() {
            out.push(target);
        }
        rest = &rest[close + 1..];
    }
    out
}

/// Points the references at the files that were actually written.
pub fn rewrite_media_refs(cards: &[AnkiCard], written: &BTreeMap<String, String>, prefix: &str) -> Vec<AnkiCard> {
    if written.is_empty() {
        return cards.to_vec();
    }
    // Longest first, so `foto.jpg` never eats the start of `foto.jpg.png`.
    let mut names: Vec<&String> = written.keys().collect();
    names.sort_by_key(|name| std::cmp::Reverse(name.len()));

    let fix = |text: &str| {
        let mut out = text.to_string();
        for name in &names {
            // Anki file names are full of spaces, and `![](a b.png)` is not an image in Markdown.
            let target = format!("{prefix}{}", written[*name]).replace(' ', "%20");
            out = out.replace(&format!("]({name})"), &format!("]({target})"));
        }
        out
    };
    cards.iter().map(|card| AnkiCard { q: fix(&card.q), a: fix(&card.a), ..card.clone() }).collect()
}

/// Splits a deck into notes of at most `size` cards: a single note of 2000 cards takes ~900 ms to
/// render every time it is opened; at 100 per note that is ~45 ms.
pub fn chunk_cards(cards: &[AnkiCard], size: usize) -> Vec<Vec<AnkiCard>> {
    if size == 0 || cards.len() <= size {
        return vec![cards.to_vec()];
    }
    cards.chunks(size).map(<[AnkiCard]>::to_vec).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: &str = "2026-07-21";

    fn card(over: CardState) -> CardState {
        over
    }

    fn base() -> CardState {
        CardState {
            file: "Sepse.md".into(),
            q: "Tríade do qSOFA?".into(),
            ease: 2.5,
            interval: 0.0,
            reps: 0,
            due: TODAY.into(),
            lapses: 0,
            suspended: false,
        }
    }

    #[test]
    fn the_local_date_is_the_calendar_date() {
        assert_eq!(today_iso(chrono::Local::now()), chrono::Local::now().date_naive().format("%Y-%m-%d").to_string());
    }

    #[test]
    fn days_are_added_across_months_and_leap_days() {
        assert_eq!(add_days("2026-07-30", 3.0), "2026-08-02");
        assert_eq!(add_days("2028-02-28", 1.0), "2028-02-29");
    }

    #[test]
    fn a_new_card_is_due_today() {
        let card = new_card("a.md", "q", TODAY);
        assert_eq!(card.due, TODAY);
        assert_eq!(card.ease, 2.5);
        assert_eq!(card.reps, 0);
    }

    #[test]
    fn again_resets_and_counts_a_lapse() {
        let graded = grade(&card(CardState { reps: 5, interval: 30.0, lapses: 1, ..base() }), Grade::Again, TODAY);
        assert_eq!(graded.reps, 0);
        assert_eq!(graded.interval, 0.0);
        assert_eq!(graded.due, TODAY);
        assert_eq!(graded.lapses, 2);
        assert!((graded.ease - 2.3).abs() < 1e-9);
    }

    #[test]
    fn the_ease_never_falls_below_the_floor() {
        let mut card = card(CardState { ease: 1.35, ..base() });
        card = grade(&card, Grade::Again, TODAY);
        card = grade(&card, Grade::Again, TODAY);
        assert!((card.ease - 1.3).abs() < 1e-9);
    }

    #[test]
    fn good_schedules_one_then_six_then_the_ease() {
        let first = grade(&base(), Grade::Good, TODAY);
        assert_eq!((first.interval, first.reps), (1.0, 1));
        assert_eq!(first.due, "2026-07-22");
        let second = grade(&card(CardState { reps: 1, interval: 1.0, ..base() }), Grade::Good, TODAY);
        assert_eq!(second.interval, 6.0);
        assert_eq!(second.due, "2026-07-27");
        let third = grade(&card(CardState { reps: 2, interval: 6.0, ..base() }), Grade::Good, TODAY);
        assert_eq!(third.interval, 15.0);
    }

    #[test]
    fn hard_grows_slowly_and_lowers_the_ease() {
        let graded = grade(&card(CardState { reps: 3, interval: 10.0, ..base() }), Grade::Hard, TODAY);
        assert!((graded.interval - 12.0).abs() < 1e-9);
        assert!((graded.ease - 2.35).abs() < 1e-9);
    }

    #[test]
    fn easy_grows_faster_and_raises_the_ease() {
        let graded = grade(&card(CardState { reps: 3, interval: 10.0, ..base() }), Grade::Easy, TODAY);
        assert!((graded.interval - 32.5).abs() < 1e-9);
        assert!((graded.ease - 2.65).abs() < 1e-9);
    }

    #[test]
    fn a_success_never_schedules_less_than_a_day() {
        let graded = grade(&card(CardState { reps: 4, interval: 0.5, ease: 1.3, ..base() }), Grade::Good, TODAY);
        assert!(graded.interval >= 1.0);
    }

    #[test]
    fn a_suspended_card_is_not_due() {
        assert!(!is_due(&card(CardState { suspended: true, ..base() }), TODAY));
        assert!(is_due(&base(), TODAY));
    }

    #[test]
    fn the_due_list_drains_oldest_first() {
        let mut file = SrsFile::default();
        file.cards.insert("b".into(), CardState { due: "2026-07-20".into(), q: "Z".into(), ..base() });
        file.cards.insert("a".into(), CardState { due: "2026-07-19".into(), q: "A".into(), ..base() });
        file.cards.insert("c".into(), CardState { due: "2026-08-01".into(), ..base() });
        let due = due_cards(&file, TODAY);
        assert_eq!(due.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn the_stats_count_what_the_panel_shows() {
        let mut file = SrsFile::default();
        file.cards.insert("a".into(), base());
        file.cards.insert("b".into(), CardState { reps: 3, suspended: true, ..base() });
        file.cards.insert("c".into(), CardState { due: "2026-08-30".into(), reps: 1, ..base() });
        let stats = stats(&file, TODAY);
        assert_eq!((stats.total, stats.due, stats.suspended, stats.fresh), (3, 1, 1, 1));
    }

    #[test]
    fn the_report_forecasts_the_next_two_weeks() {
        let mut file = SrsFile::default();
        file.cards.insert("a".into(), CardState { reps: 2, ..base() });
        file.cards.insert("b".into(), CardState { due: "2026-07-22".into(), ..base() });
        let report = report(&file, TODAY);
        assert_eq!(report.forecast.len(), 14);
        assert_eq!(report.forecast[0], (TODAY.to_string(), 1), "the backlog is today's column");
        assert_eq!(report.forecast[1].1, 1);
        assert!((report.average_ease - 2.5).abs() < 1e-9);
    }

    #[test]
    fn syncing_adds_new_cards_and_prunes_the_gone_ones() {
        let mut srs = SrsFile::default();
        srs.cards.insert("old".into(), CardState { file: "a.md".into(), q: "Velha".into(), ..base() });
        let found = vec![("new".to_string(), "Nova".to_string())];
        let (next, added, removed) = sync_file(&srs, "a.md", &found, TODAY);
        assert_eq!((added, removed), (1, 1));
        assert!(next.cards.contains_key("new") && !next.cards.contains_key("old"));
    }

    #[test]
    fn syncing_keeps_the_schedule_of_a_card_whose_answer_changed() {
        let mut srs = SrsFile::default();
        srs.cards.insert("same".into(), CardState { reps: 7, ease: 2.1, ..CardState { file: "a.md".into(), q: "P".into(), ..base() } });
        let (next, added, removed) = sync_file(&srs, "a.md", &[("same".to_string(), "P".to_string())], TODAY);
        assert_eq!((added, removed), (0, 0));
        assert_eq!(next.cards["same"].reps, 7);
    }

    #[test]
    fn a_vault_wide_sync_does_not_touch_a_note_it_did_not_read() {
        let mut srs = SrsFile::default();
        srs.cards.insert("keep".into(), CardState { file: "ilegivel.md".into(), ..base() });
        let mut found = BTreeMap::new();
        found.insert("a.md".to_string(), vec![("id".to_string(), "P".to_string())]);
        let next = sync_vault(&srs, &found, TODAY);
        assert!(next.cards.contains_key("keep"), "the unread note keeps its schedule");
        assert!(next.cards.contains_key("id"));
    }

    #[test]
    fn the_file_round_trips_and_keeps_the_field_order() {
        let mut file = SrsFile::default();
        file.cards.insert("abcdef0123456789".into(), CardState { suspended: true, ..base() });
        let directory = std::env::temp_dir().join(format!("localobsidian-srs-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let path = directory.join("srs.json");
        save(&path, &file).expect("save");

        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.starts_with("{\n  \"version\": 1,\n  \"cards\": {"), "{text}");
        assert!(text.contains("\"file\": \"Sepse.md\""), "{text}");
        assert!(text.contains("\"suspended\": true"), "{text}");
        let (again, dropped) = load(&path);
        assert_eq!(dropped, 0);
        assert_eq!(again, file);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_file_written_by_the_phone_is_read_as_is() {
        // The Android app writes `interval` as a Double (`1.0`), the desktop as a Number (`1`), and
        // only `suspended: true` is written at all. Both shapes must land on the same schedule.
        let directory = std::env::temp_dir().join(format!("localobsidian-srs-android-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let path = directory.join("srs.json");
        std::fs::write(
            &path,
            r#"{"version":1,"cards":{"aa11bb22cc33dd44":{"file":"Patologias/Sepse.md","q":"Tríade do qSOFA?","ease":2.3,"interval":1.0,"reps":1,"due":"2026-07-22","lapses":0},"ee55ff66aa77bb88":{"file":"Protocolo.md","q":"Primeira hora?","ease":2.5,"interval":0.0,"reps":0,"due":"2026-07-21","lapses":0,"suspended":true}}}"#,
        )
        .expect("write");

        let (file, dropped) = load(&path);
        assert_eq!(dropped, 0);
        assert_eq!(file.version, 1);
        assert_eq!(file.cards.len(), 2);
        let card = &file.cards["aa11bb22cc33dd44"];
        assert_eq!((card.ease, card.interval, card.reps), (2.3, 1.0, 1));
        assert!(!card.suspended);
        assert!(file.cards["ee55ff66aa77bb88"].suspended);

        // And what we write back is read by the phone: same keys, `suspended` only when true.
        save(&path, &file).expect("save");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("\"interval\": 1.0"), "{text}");
        assert_eq!(text.matches("\"suspended\": true").count(), 1, "{text}");
        let (again, _) = load(&path);
        assert_eq!(again, file);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_unreadable_file_is_an_empty_schedule() {
        let path = std::env::temp_dir().join(format!("localobsidian-srs-bad-{}.json", std::process::id()));
        std::fs::write(&path, "isto não é json").expect("write");
        let (file, dropped) = load(&path);
        assert!(file.cards.is_empty());
        assert_eq!(dropped, 0);
        let _ = std::fs::remove_file(&path);
        assert!(load(Path::new("nao-existe.json")).0.cards.is_empty());
    }

    #[test]
    fn a_card_with_an_absolute_path_is_dropped() {
        let path = std::env::temp_dir().join(format!("localobsidian-srs-abs-{}.json", std::process::id()));
        std::fs::write(
            &path,
            r#"{"version":1,"cards":{"a":{"file":"C:/vault/Nota.md","q":"P","ease":2.5,"interval":0,"reps":0,"due":"2026-07-21","lapses":0}}}"#,
        )
        .expect("write");
        let (file, dropped) = load(&path);
        assert_eq!(dropped, 1);
        assert!(file.cards.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn anki_text_goes_out_tab_separated() {
        let text = to_anki_text(&[("Pergunta?\tcom tab".to_string(), "linha\nquebrada".to_string())]);
        assert_eq!(text, "Pergunta? com tab\tlinha<br>quebrada");
    }

    #[test]
    fn an_anki_text_export_comes_back_as_cards() {
        let text = "#separator:tab\n#tags column:3\nPergunta?\tResposta\tcardio uti\nCom <img src=\"a.png\">\tOutra\t";
        let import = from_anki_text(text);
        assert_eq!(import.cards.len(), 2);
        assert_eq!(import.cards[0].q, "Pergunta?");
        assert_eq!(import.cards[0].a, "Resposta");
        assert_eq!(import.cards[0].tags, ["cardio", "uti"]);
        assert_eq!(import.with_media, 1);
    }

    #[test]
    fn a_cloze_card_without_an_extra_field_is_kept() {
        let import = from_anki_text("A {{c1::mitocôndria}} produz ATP\t\tbio");
        assert_eq!(import.cards.len(), 1);
        assert!(import.cards[0].cloze);
        assert_eq!(import.cards[0].q, "A ==mitocôndria== produz ATP");
    }

    #[test]
    fn anki_fields_become_markdown() {
        assert_eq!(anki_field_to_markdown("{{c1::ATP}} é energia", false), "==ATP== é energia");
        assert_eq!(anki_field_to_markdown("[$]x^2[/$]", false), "$x^2$");
        assert_eq!(anki_field_to_markdown("a<br>b", false), "a b");
        assert_eq!(anki_field_to_markdown("<b>forte</b>", false), "forte");
        assert_eq!(anki_field_to_markdown("&amp; &nbsp; fim", false), "& fim");
        assert_eq!(anki_field_to_markdown("[sound:a.mp3]x", true), "[a.mp3](a.mp3)x");
    }

    #[test]
    fn an_anki_deck_becomes_a_note_of_callouts() {
        let cards = vec![
            AnkiCard { q: "P1".into(), a: "R1".into(), tags: vec!["cardio".into()], cloze: false },
            AnkiCard { q: "A ==x== B".into(), a: String::new(), tags: Vec::new(), cloze: true },
        ];
        let note = anki_to_markdown("Deck", &cards);
        assert!(note.starts_with("# Deck\n\n#cardio\n\n"), "{note}");
        assert!(note.contains("> [!card]- P1\n> R1"), "{note}");
        assert!(note.contains("> [!card]\n> A ==x== B"), "{note}");
    }

    #[test]
    fn media_references_are_collected_and_rewritten() {
        let cards = vec![AnkiCard { q: "![](foto um.png)".into(), a: "![](https://x/y.png)".into(), tags: Vec::new(), cloze: false }];
        assert_eq!(collect_media_refs(&cards), ["foto um.png"]);
        let mut written = BTreeMap::new();
        written.insert("foto um.png".to_string(), "foto-um.png".to_string());
        let fixed = rewrite_media_refs(&cards, &written, "_attachments/");
        assert_eq!(fixed[0].q, "![](_attachments/foto-um.png)");
    }

    #[test]
    fn a_deck_is_split_into_notes_of_a_hundred() {
        let cards: Vec<AnkiCard> = (0..250).map(|i| AnkiCard { q: format!("P{i}"), a: "R".into(), tags: Vec::new(), cloze: false }).collect();
        let chunks = chunk_cards(&cards, 100);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[2].len(), 50);
    }
}
