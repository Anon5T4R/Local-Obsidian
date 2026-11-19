//! The review session and the statistics panel.
//!
//! Port of the upstream `ReviewPanel`/`ReviewStatsPanel`: a deck (everything, a tag, or the open
//! note), one card at a time, `Space` to reveal and `1`-`4` to grade - the whole session is meant to
//! be keyboard-only. The answer is read from the note on every reveal, so editing it shows up on the
//! next review without any re-sync.
//!
//! Practice is not review: repeating this session's cards, or studying ahead, never touches the
//! schedule.
//!
//! The panel always names the deck it is on ("MOC - Medicina", "#uti", "the whole vault") and how
//! many cards that deck has: a session that says 4598 while the reader asked for one note is a lie,
//! and the numbers come from the deck, never from the vault.

use localobsidian_review as review;

use crate::i18n;
use crate::tl;
use crate::vault_state::VaultState;

/// Which cards the session covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deck {
    /// Every card of the vault.
    All,
    /// The notes carrying a tag.
    Tag(String),
    /// The note that is open, by its path inside the vault.
    Note(String),
}

impl Deck {
    /// How the deck is named in the panel: the note's own name, the tag as written, or the vault.
    fn label(&self, state: &VaultState) -> String {
        match self {
            Deck::All => tl!("the whole vault").to_string(),
            Deck::Tag(tag) => format!("#{tag}"),
            Deck::Note(relative) => {
                state.files.iter().find(|file| &file.relative_path == relative).map(|file| file.name.clone()).unwrap_or_else(|| relative.clone())
            }
        }
    }
}

/// What the deck holds right now: how many cards, how many are due today, and when the next one
/// comes up.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Scope {
    cards: usize,
    due: usize,
    /// The closest date after today, when nothing else is due.
    next_due: Option<String>,
}

/// A deck the panel can switch to, and how many cards it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeckOption {
    pub deck: Deck,
    pub label: String,
    pub cards: usize,
}

/// The decks the vault actually has: the whole vault, the note that is open (when it has cards), and
/// the tags that have cards. A deck nobody can review is not offered.
fn decks(state: &VaultState) -> Vec<DeckOption> {
    // Cards per note file, once: the tags sum this instead of walking the schedule for each tag.
    let mut per_file: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for card in state.srs.cards.values() {
        if !card.file.is_empty() {
            *per_file.entry(card.file.as_str()).or_default() += 1;
        }
    }
    let cards_of = |relative: &str| per_file.get(relative).copied().unwrap_or(0);

    let mut options = vec![DeckOption { deck: Deck::All, label: tl!("the whole vault").to_string(), cards: per_file.values().sum() }];
    if let Some(active) = state.active.as_ref().and_then(|path| state.files.iter().find(|file| &file.path == path))
        && cards_of(&active.relative_path) > 0
    {
        options.push(DeckOption {
            deck: Deck::Note(active.relative_path.clone()),
            label: active.name.clone(),
            cards: cards_of(&active.relative_path),
        });
    }

    let mut tags: Vec<DeckOption> = state
        .index
        .tags
        .iter()
        .filter_map(|(tag, names)| {
            let cards: usize = names
                .iter()
                .filter_map(|name| state.files.iter().find(|file| file.name.eq_ignore_ascii_case(name)))
                .map(|file| cards_of(&file.relative_path))
                .sum();
            (cards > 0).then(|| DeckOption { deck: Deck::Tag(tag.clone()), label: format!("#{tag}"), cards })
        })
        .collect();
    tags.sort_by(|a, b| b.cards.cmp(&a.cards).then_with(|| a.label.cmp(&b.label)));
    options.extend(tags);
    options
}

/// A card's question as one line of plain text: no markup, no more than `limit` characters.
fn plain(question: &str, limit: usize) -> String {
    let blocks = localobsidian_md::preview::blocks(&localobsidian_md::preview::prepare(question));
    let text = blocks.first().map(|block| block.plain()).unwrap_or_default();
    let text = text.trim().replace('\n', " ");
    let mut out: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        out.push('…');
    }
    out
}

/// How many cards of the session the statistics panel lists.
const SESSION_LIST: usize = 8;

/// The review session's state.
pub struct Review {
    pub open: bool,
    pub stats_open: bool,
    pub deck: Deck,
    /// Cards waiting in this session, the current one first.
    queue: Vec<(String, review::CardState)>,
    revealed: bool,
    /// Cards answered in this session (never the practice ones), so they can be replayed without
    /// touching the schedule - practice is not the same thing as reviewing.
    seen: Vec<(String, review::CardState)>,
    done: usize,
    /// Repeating or studying ahead grades nothing.
    practice: bool,
    /// The deck as it was when the session started: what the header reports.
    scope: Scope,
}

impl Default for Review {
    fn default() -> Self {
        Self {
            open: false,
            stats_open: false,
            deck: Deck::All,
            queue: Vec::new(),
            revealed: false,
            seen: Vec::new(),
            done: 0,
            practice: false,
            scope: Scope::default(),
        }
    }
}

impl Review {
    /// Opens a session for `deck`, with the cards due today.
    pub fn start(&mut self, state: &VaultState, deck: Deck) {
        self.deck = deck;
        self.practice = false;
        self.open = true;
        self.reload(state, 0);
    }

    /// Fills the queue from the schedule.
    fn reload(&mut self, state: &VaultState, ahead_days: i64) {
        let today = review::today();
        let when = if ahead_days == 0 { today.clone() } else { review::date_in(ahead_days) };
        let allowed = self.deck_files(state);
        self.queue = review::due_cards(&state.srs, &when)
            .into_iter()
            .filter(|(_, card)| allowed.as_ref().is_none_or(|files| files.contains(&card.file)))
            .collect();
        self.scope = self.scope_of(state, &today);
        self.revealed = false;
        self.done = 0;
    }

    /// How many cards the deck holds, how many of them are due today, and when the next one is.
    fn scope_of(&self, state: &VaultState, today: &str) -> Scope {
        let allowed = self.deck_files(state);
        let cards: Vec<&review::CardState> =
            state.srs.cards.values().filter(|card| allowed.as_ref().is_none_or(|files| files.contains(&card.file))).collect();
        Scope {
            cards: cards.len(),
            due: cards.iter().filter(|card| review::is_due(card, today)).count(),
            next_due: cards.iter().filter(|card| !card.suspended && card.due.as_str() > today).map(|card| card.due.clone()).min(),
        }
    }

    /// The relative paths this deck covers; `None` means every note.
    fn deck_files(&self, state: &VaultState) -> Option<Vec<String>> {
        match &self.deck {
            Deck::All => None,
            Deck::Note(relative) => Some(vec![relative.clone()]),
            Deck::Tag(tag) => {
                let notes = state.index.tags.get(tag)?;
                Some(
                    notes
                        .iter()
                        .filter_map(|name| state.files.iter().find(|file| file.name.eq_ignore_ascii_case(name)))
                        .map(|file| file.relative_path.clone())
                        .collect(),
                )
            }
        }
    }

    /// Reopens the session with the cards due today.
    pub fn restart(&mut self, state: &VaultState) {
        self.practice = false;
        self.reload(state, 0);
    }

    fn current(&self) -> Option<&(String, review::CardState)> {
        self.queue.first()
    }

    /// How many cards the session has left (the tests read the queue without drawing it).
    #[cfg(test)]
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// How many cards the deck holds, as the header reports it.
    #[cfg(test)]
    pub fn scope_cards(&self) -> usize {
        self.scope.cards
    }

    /// Reveals the current card's answer.
    pub fn reveal(&mut self) {
        if self.current().is_some() {
            self.revealed = true;
        }
    }

    /// Grades the current card. `again` keeps it in this session, at the back of the queue.
    pub fn answer(&mut self, state: &mut VaultState, grade: review::Grade) {
        let Some((id, card)) = self.current().cloned() else { return };
        if !self.practice {
            state.grade_card(&id, grade);
        }
        if grade == review::Grade::Again {
            let first = self.queue.remove(0);
            self.queue.push(first);
        } else {
            self.queue.remove(0);
            self.done += 1;
            if !self.practice && !self.seen.iter().any(|(seen, _)| seen == &id) {
                self.seen.push((id, card));
            }
        }
        self.revealed = false;
    }

    /// Parks the current card.
    pub fn suspend(&mut self, state: &mut VaultState) {
        let Some((id, _)) = self.current().cloned() else { return };
        state.suspend_card(&id, true);
        self.queue.remove(0);
        self.revealed = false;
    }

    /// Replays the cards answered in this session, without grading them again.
    fn repeat(&mut self) {
        if self.seen.is_empty() {
            return;
        }
        self.practice = true;
        self.done = 0;
        self.revealed = false;
        self.queue = self.seen.clone();
    }

    /// Studies the whole deck ahead of the schedule: nothing is graded.
    fn practice_all(&mut self, state: &VaultState) {
        self.practice = true;
        self.revealed = false;
        self.done = 0;
        let allowed = self.deck_files(state);
        let mut cards: Vec<(String, review::CardState)> = state
            .srs
            .cards
            .iter()
            .filter(|(_, card)| !card.suspended && allowed.as_ref().is_none_or(|files| files.contains(&card.file)))
            .map(|(id, card)| (id.clone(), card.clone()))
            .collect();
        cards.sort_by(|a, b| a.1.due.cmp(&b.1.due).then_with(|| a.1.q.cmp(&b.1.q)));
        self.queue = cards;
    }

    /// The keys the session answers: `Space` reveals, `1`-`4` grade, `Escape` closes.
    pub fn handle_keys(&mut self, state: &mut VaultState, ctx: &egui::Context) -> bool {
        if !self.open || self.stats_open {
            return false;
        }
        // Never while the note has the keyboard: a digit typed into a note must not grade a card.
        if ctx.memory(|memory| memory.focused().is_some()) {
            return false;
        }
        let mut wake = false;
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.open = false;
            return true;
        }
        if self.current().is_none() {
            return false;
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Space)) {
            self.reveal();
            wake = true;
        }
        if self.revealed {
            for (key, grade) in [
                (egui::Key::Num1, review::Grade::Again),
                (egui::Key::Num2, review::Grade::Hard),
                (egui::Key::Num3, review::Grade::Good),
                (egui::Key::Num4, review::Grade::Easy),
            ] {
                if ctx.input(|input| input.key_pressed(key)) {
                    self.answer(state, grade);
                    wake = true;
                    break;
                }
            }
        }
        wake
    }

    /// Draws the session.
    pub fn ui(&mut self, ui: &mut egui::Ui, state: &mut VaultState, images: &mut crate::images::Images) {
        // The header Restart button and the menu item share the same reload.
        ui.add_space(4.0);
        // What the session is about is a control, not a caption: the deck can be changed from here,
        // and the list only holds decks the vault actually has.
        let mut chosen: Option<Deck> = None;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Review")).strong());
            let combo = egui::ComboBox::from_id_salt("review-deck")
                .selected_text(egui::RichText::new(self.deck.label(state)).strong())
                .width(190.0)
                .show_ui(ui, |ui| {
                    for option in decks(state) {
                        let label = format!("{} ({})", option.label, option.cards);
                        if ui.selectable_label(option.deck == self.deck, label).clicked() {
                            chosen = Some(option.deck);
                        }
                    }
                });
            // A combo box with no label is a widget a screen reader cannot announce; this one is
            // the deck of the session.
            combo.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, tl!("Deck")));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::panels::close_x(ui, tl!("Close")).clicked() {
                    self.open = false;
                }
                // The statistics panel is one panel: this button opens and closes it, and shows
                // whether it is open, instead of stacking a second copy of the same thing.
                if ui
                    .selectable_label(self.stats_open, egui::RichText::new(tl!("Statistics")).small())
                    .on_hover_text(tl!("The review statistics of this deck"))
                    .clicked()
                {
                    self.stats_open = !self.stats_open;
                }
                if ui.small_button(tl!("Restart")).on_hover_text(tl!("Reload the queue with what is due today")).clicked() {
                    self.restart(state);
                }
            });
        });
        if let Some(deck) = chosen {
            // Another deck is another session: the queue and the counts start over.
            self.start(state, deck);
        }
        let counts = [
            i18n::trn(self.scope.cards as u64, "{n} card in this deck", "{n} cards in this deck"),
            i18n::tr_fmt("{due} due today", &[("due", &self.scope.due.to_string())]),
            i18n::tr_fmt("{done} done in this session", &[("done", &self.done.to_string())]),
        ]
        .join(" · ");
        ui.label(egui::RichText::new(counts).small().weak());
        if self.practice {
            ui.label(egui::RichText::new(tl!("practice - nothing is graded")).small().color(ui.visuals().warn_fg_color));
        }
        ui.separator();

        let Some((id, card)) = self.current().cloned() else {
            self.empty(ui, state);
            return;
        };

        // The grade row keeps its room: a scroll area that grows to the whole panel would push the
        // buttons out of sight, and the session is meant to be answerable with one hand on the keys.
        let card_height = (ui.available_height() - 44.0).max(80.0);

        egui::ScrollArea::vertical().id_salt("review-card").max_height(card_height).auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal(|ui| {
                let source = ui.add(egui::Label::new(egui::RichText::new(&card.file).small().weak()).sense(egui::Sense::click()));
                if source.on_hover_text(tl!("Open the note this card lives in")).clicked()
                    && let Some(note) = state.note_for_card(&card.file)
                {
                    let path = note.path.clone();
                    state.open_note(&path);
                }
            });
            ui.add_space(6.0);
            // The card is written in the note's Markdown: draw it the way the reading view draws the
            // note. A card is a sentence, not a syntax lesson.
            crate::editor::markdown(ui, state, images, &card.q, 17.0);
            ui.add_space(8.0);
            if self.revealed {
                let answer = state.card_answer(&id).unwrap_or_default();
                if answer.trim().is_empty() {
                    ui.label(egui::RichText::new(tl!("this card has no answer")).weak());
                } else {
                    egui::Frame::new().fill(ui.visuals().faint_bg_color).inner_margin(8.0).corner_radius(4.0).show(ui, |ui| {
                        crate::editor::markdown(ui, state, images, &answer, 15.0);
                    });
                }
            } else if ui.button(i18n::tr_fmt("Reveal ({key})", &[("key", tl!("Space"))])).clicked() {
                self.revealed = true;
            }
        });

        ui.add_space(4.0);
        // The actions of the card live together, after the answer: grading and parking are the same
        // decision - what to do with this card now.
        if self.revealed {
            ui.horizontal(|ui| {
                for (label, key, grade) in [
                    (tl!("Again"), "1", review::Grade::Again),
                    (tl!("Hard"), "2", review::Grade::Hard),
                    (tl!("Good"), "3", review::Grade::Good),
                    (tl!("Easy"), "4", review::Grade::Easy),
                ] {
                    if ui.button(format!("{label} ({key})")).clicked() {
                        self.answer(state, grade);
                    }
                }
                ui.separator();
                if ui.small_button(tl!("Suspend")).on_hover_text(tl!("Stop asking this card")).clicked() {
                    self.suspend(state);
                }
            });
        }
    }

    /// What the panel shows when the queue is empty.
    ///
    /// It always says which deck it is talking about: "nothing is due" means one thing in a note
    /// with three cards and another in a vault with four thousand.
    fn empty(&mut self, ui: &mut egui::Ui, state: &VaultState) {
        ui.add_space(12.0);
        let deck = self.deck.label(state);
        if self.done > 0 {
            ui.label(egui::RichText::new(i18n::tr_fmt("done for now - {count} reviewed", &[("count", &self.done.to_string())])).size(15.0));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if !self.seen.is_empty()
                    && ui.button(i18n::tr_fmt("Repeat the {count} of this session", &[("count", &self.seen.len().to_string())])).clicked()
                {
                    self.repeat();
                }
                if ui.button(tl!("Close the panel")).clicked() {
                    self.open = false;
                }
            });
            return;
        }

        if self.scope.cards == 0 {
            let message = match self.deck {
                Deck::All => tl!("this vault has no cards yet").to_string(),
                Deck::Tag(_) => i18n::tr_fmt("no cards under {deck} yet", &[("deck", &deck)]),
                Deck::Note(_) => i18n::tr_fmt("{deck} has no cards yet", &[("deck", &deck)]),
            };
            ui.label(egui::RichText::new(message).size(15.0));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tl!("a card is a callout in the note itself:")).small().weak());
            egui::Frame::new().fill(ui.visuals().code_bg_color).inner_margin(8.0).corner_radius(4.0).show(ui, |ui| {
                ui.label(egui::RichText::new(tl!("> [!card] the question")).monospace().small());
                ui.label(egui::RichText::new(tl!("> the answer, under it")).monospace().small());
            });
            ui.add_space(4.0);
            ui.label(egui::RichText::new(tl!("one card per callout; ==highlights== in the answer become cloze gaps")).small().weak());
            return;
        }

        // Cards exist, none is due today: say how many, and when the next one comes up.
        let message = i18n::tr_fmt("{cards} cards in {deck}, none due today", &[("cards", &self.scope.cards.to_string()), ("deck", &deck)]);
        ui.label(egui::RichText::new(message).size(15.0));
        ui.add_space(4.0);
        if let Some(next) = &self.scope.next_due {
            let days = days_until(next).unwrap_or(0);
            let when = if days <= 1 {
                i18n::tr_fmt("next tomorrow ({date})", &[("date", next)])
            } else {
                i18n::tr_fmt("next in {days} days ({date})", &[("days", &days.to_string()), ("date", next)])
            };
            ui.label(egui::RichText::new(when).small().weak());
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .button(i18n::trn(self.scope.cards as u64, "Practice the {n} card", "Practice the {n} cards"))
                .on_hover_text(tl!("Study ahead of the schedule; nothing is graded"))
                .clicked()
            {
                self.practice_all(state);
            }
            if !self.seen.is_empty()
                && ui.button(i18n::tr_fmt("Repeat the {count} of this session", &[("count", &self.seen.len().to_string())])).clicked()
            {
                self.repeat();
            }
        });
    }

    /// Draws the statistics panel, for the deck the session is on.
    pub fn stats_ui(&mut self, ui: &mut egui::Ui, state: &mut VaultState) {
        let files = self.deck_files(state);
        let report = state.srs_report_for(files.as_deref());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Review statistics")).strong());
            // Same rule as the session header: the numbers belong to a named deck.
            ui.label(egui::RichText::new(self.deck.label(state)).small().weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::panels::close_x(ui, tl!("Close")).clicked() {
                    self.stats_open = false;
                }
            });
        });
        ui.separator();

        egui::ScrollArea::vertical().id_salt("review-stats").auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("srs-numbers").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                for (label, value) in [
                    (tl!("Cards"), report.stats.total.to_string()),
                    (tl!("Due today"), report.stats.due.to_string()),
                    (tl!("New"), report.stats.fresh.to_string()),
                    (tl!("Suspended"), report.stats.suspended.to_string()),
                    (tl!("Learned"), report.learned.to_string()),
                    (tl!("Retention"), format!("{:.0}%", report.retention * 100.0)),
                    (tl!("Average ease"), format!("{:.2}", report.average_ease)),
                ] {
                    ui.label(label);
                    ui.label(egui::RichText::new(value).strong());
                    ui.end_row();
                }
            });

            // The session's own queue, with the note behind each card: seeing what is coming is how
            // the reader decides whether to keep going, and where the card came from.
            if self.queue.len() > 1 {
                ui.add_space(10.0);
                ui.label(egui::RichText::new(tl!("Next in this session")).strong());
                for (_, card) in self.queue.iter().skip(1).take(SESSION_LIST) {
                    let note = state
                        .files
                        .iter()
                        .find(|file| file.relative_path == card.file)
                        .map(|file| file.name.clone())
                        .unwrap_or_else(|| card.file.clone());
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(note).small().weak());
                        ui.label(egui::RichText::new(plain(&card.q, 56)).small());
                    });
                }
            }

            ui.add_space(10.0);
            ui.label(egui::RichText::new(tl!("Next 14 days")).strong());
            for (date, count) in &report.forecast {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(date).monospace().small());
                    // A bar makes the shape of the fortnight readable at a glance.
                    let width = (f32::min(*count as f32, 40.0) / 40.0) * 120.0;
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(120.0, 10.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, ui.visuals().faint_bg_color);
                    if width > 0.0 {
                        let bar = egui::Rect::from_min_size(rect.min, egui::vec2(width, rect.height()));
                        ui.painter().rect_filled(bar, 2.0, ui.visuals().selection.bg_fill);
                    }
                    ui.label(egui::RichText::new(count.to_string()).small());
                });
            }

            ui.add_space(10.0);
            if !report.top_files.is_empty() {
                // The number is a count of notes, not a stray figure beside a heading.
                let mut notes = std::collections::BTreeSet::new();
                for card in state.srs.cards.values() {
                    if card.file.is_empty() || files.as_ref().is_some_and(|files| !files.contains(&card.file)) {
                        continue;
                    }
                    notes.insert(card.file.as_str());
                }
                ui.label(egui::RichText::new(tl!("Notes with the most cards")).strong());
                ui.label(egui::RichText::new(i18n::trn(notes.len() as u64, "{n} note with cards here", "{n} notes with cards here")).small().weak());
                let mut open: Option<std::path::PathBuf> = None;
                for (file, count) in &report.top_files {
                    ui.horizontal(|ui| {
                        if ui.add(egui::Label::new(egui::RichText::new(file).small()).sense(egui::Sense::click())).clicked() {
                            open = state.note_for_card(file).map(|note| note.path.clone());
                        }
                        ui.label(egui::RichText::new(count.to_string()).small().weak());
                    });
                }
                if let Some(path) = open {
                    state.open_note(&path);
                }
            }
        });
    }
}

/// How many days from today to `iso`.
fn days_until(iso: &str) -> Option<i64> {
    let today = chrono::NaiveDate::parse_from_str(&review::today(), "%Y-%m-%d").ok()?;
    let target = chrono::NaiveDate::parse_from_str(iso, "%Y-%m-%d").ok()?;
    Some((target - today).num_days())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// A vault in memory: notes with their text, and the cards the schedule holds for them.
    fn vault(notes: &[(&str, &str)], cards: &[(&str, &str)]) -> VaultState {
        let mut state = VaultState::default();
        state.files = notes
            .iter()
            .map(|(relative, _)| localobsidian_vault::NoteFile {
                name: relative.rsplit('/').next().unwrap_or(relative).trim_end_matches(".md").to_string(),
                path: PathBuf::from(format!("/v/{relative}")),
                relative_path: (*relative).to_string(),
                mtime_ms: 0,
            })
            .collect();
        let contents: BTreeMap<PathBuf, String> =
            notes.iter().map(|(relative, text)| (PathBuf::from(format!("/v/{relative}")), (*text).to_string())).collect();
        state.index = localobsidian_vault::index::build(&state.files, &contents);
        state.replace_contents_for_tests(contents);
        let today = review::today();
        for (file, question) in cards {
            let card = review::new_card(file, question, &today);
            state.srs.cards.insert(review::card_id(file, question, ""), card);
        }
        state
    }

    #[test]
    fn the_selector_lists_only_the_decks_the_vault_has() {
        let state = vault(
            &[
                ("MOC.md", "---\ntags: [uti]\n---\n\n# Moc\n"),
                ("Outra.md", "---\ntags: [uti]\n---\n\n# Outra\n"),
                ("Vazia.md", "---\ntags: [sem-cards]\n---\n\n# Vazia\n"),
            ],
            &[("MOC.md", "pergunta do MOC"), ("Outra.md", "primeira"), ("Outra.md", "segunda")],
        );
        let mut state = state;
        state.active = Some(state.files[0].path.clone());

        let options = decks(&state);
        assert_eq!(options[0].deck, Deck::All, "the whole vault comes first");
        assert_eq!(options[0].cards, 3);
        assert!(options.iter().any(|option| option.deck == Deck::Note("MOC.md".to_string()) && option.cards == 1), "{options:?}");
        // Only the tag that has cards is offered, and a tag never appears twice.
        assert!(options.iter().any(|option| option.deck == Deck::Tag("uti".to_string()) && option.cards == 3), "{options:?}");
        assert!(!options.iter().any(|option| option.label == "#sem-cards"), "a tag with no cards is not a deck: {options:?}");
    }

    #[test]
    fn a_note_without_cards_is_not_a_deck() {
        let mut state = vault(&[("MOC.md", "# Moc\n"), ("Outra.md", "# Outra\n")], &[("Outra.md", "pergunta")]);
        state.active = Some(state.files[0].path.clone());
        let options = decks(&state);
        assert!(!options.iter().any(|option| matches!(&option.deck, Deck::Note(_))), "{options:?}");
    }

    #[test]
    fn the_session_list_shows_the_question_without_its_markup() {
        assert_eq!(plain("A **tetralogia** e o `CIA` são o quê?", 60), "A tetralogia e o CIA são o quê?");
        assert!(plain(&"palavra ".repeat(20), 20).ends_with('…'), "a long question is cut");
    }
}
