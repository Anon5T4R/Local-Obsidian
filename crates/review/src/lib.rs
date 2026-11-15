//! Spaced repetition: the SM-2 schedule, the cards a note declares, and the file the phone reads.
//!
//! Port of the upstream `srs.ts` and `cards.ts`. Two things here are contracts, not choices:
//!
//! - **The schedule maths** is SM-2 exactly as the desktop and the Android app run it: the same
//!   `.openobsidian/srs.json` is read by either, so the three must agree on every number.
//! - **The card id** is a hash of the note's relative path and the question, computed over UTF-16
//!   code units (what JavaScript's `charCodeAt` returns). Hashing bytes instead would give the same
//!   card two ids, one per app, and the schedule would fork.
//!
//! A card lives inside a note as a callout with a `==highlight==`: `> [!card]- Question` with the
//! answer in the body, or a cloze sentence with one gap per highlight. The note stays readable in
//! any Markdown editor and the cards disappear into plain prose when the review engine is not there.

pub mod cards;
pub mod srs;

pub use cards::{CLOZE_GAP, CardKind, ExtractedCard, card_id, cloze_parts, extract_cards, render_cloze};
pub use srs::{
    AnkiCard, AnkiImport, CardState, Grade, SrsFile, SrsReport, SrsStats, add_days, anki_field_to_markdown, anki_to_markdown, chunk_cards,
    collect_media_refs, date_in, due_cards, from_anki_text, grade, is_due, new_card, report, rewrite_media_refs, stats, sync_file, sync_vault,
    to_anki_text, today, today_iso,
};
