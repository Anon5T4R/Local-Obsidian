//! The review session and the statistics panel.
//!
//! Port of the upstream `ReviewPanel`/`ReviewStatsPanel`: a deck (everything, a tag, or the open
//! note), one card at a time, `Space` to reveal and `1`-`4` to grade - the whole session is meant to
//! be keyboard-only. The answer is read from the note on every reveal, so editing it shows up on the
//! next review without any re-sync.
//!
//! Practice is not review: repeating this session's cards, or studying ahead, never touches the
//! schedule.

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
    /// The note that is open.
    Note(String),
}

impl Deck {
    fn label(&self) -> String {
        match self {
            Deck::All => tl!("the whole vault").to_string(),
            Deck::Tag(tag) => format!("#{tag}"),
            Deck::Note(name) => name.clone(),
        }
    }
}

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
}

impl Default for Review {
    fn default() -> Self {
        Self { open: false, stats_open: false, deck: Deck::All, queue: Vec::new(), revealed: false, seen: Vec::new(), done: 0, practice: false }
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
        let when = if ahead_days == 0 { today } else { review::date_in(ahead_days) };
        let allowed = self.deck_files(state);
        self.queue = review::due_cards(&state.srs, &when)
            .into_iter()
            .filter(|(_, card)| allowed.as_ref().is_none_or(|files| files.contains(&card.file)))
            .collect();
        self.revealed = false;
        self.done = 0;
    }

    /// The relative paths this deck covers; `None` means every note.
    fn deck_files(&self, state: &VaultState) -> Option<Vec<String>> {
        match &self.deck {
            Deck::All => None,
            Deck::Note(name) => Some(vec![name.clone()]),
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

    /// Pulls in what comes due over the next week, without grading it.
    fn ahead(&mut self, state: &VaultState) {
        self.practice = true;
        self.reload(state, 7);
        let seen = self.seen.clone();
        self.queue.retain(|(id, _)| !seen.iter().any(|(seen, _)| seen == id));
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
    pub fn ui(&mut self, ui: &mut egui::Ui, state: &mut VaultState) {
        // The header Restart button and the menu item share the same reload.
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Review")).strong());
            ui.label(egui::RichText::new(self.deck.label()).small().weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("x").on_hover_text(tl!("Close")).clicked() {
                    self.open = false;
                }
                if ui.small_button(tl!("Statistics")).on_hover_text(tl!("The vault's review statistics")).clicked() {
                    self.stats_open = true;
                }
                if ui.small_button(tl!("Restart")).on_hover_text(tl!("Reload the queue with what is due today")).clicked() {
                    self.restart(state);
                }
            });
        });
        ui.label(
            egui::RichText::new(i18n::tr_fmt(
                "{left} to review, {done} done",
                &[("left", &self.queue.len().to_string()), ("done", &self.done.to_string())],
            ))
            .small()
            .weak(),
        );
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
            ui.label(egui::RichText::new(&card.q).size(17.0));
            ui.add_space(8.0);
            if self.revealed {
                let answer = state.card_answer(&id).unwrap_or_default();
                if answer.trim().is_empty() {
                    ui.label(egui::RichText::new(tl!("this card has no answer")).weak());
                } else {
                    egui::Frame::new().fill(ui.visuals().faint_bg_color).inner_margin(8.0).corner_radius(4.0).show(ui, |ui| {
                        ui.label(egui::RichText::new(&answer).size(15.0));
                    });
                }
            } else if ui.button(i18n::tr_fmt("Reveal ({key})", &[("key", tl!("Space"))])).clicked() {
                self.revealed = true;
            }
        });

        ui.add_space(4.0);
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
                if ui.small_button(tl!("Suspend")).on_hover_text(tl!("Stop asking this card")).clicked() {
                    self.suspend(state);
                }
            });
        } else if ui.small_button(tl!("Suspend")).clicked() {
            self.suspend(state);
        }
    }

    /// What the panel shows when the queue is empty.
    fn empty(&mut self, ui: &mut egui::Ui, state: &VaultState) {
        ui.add_space(12.0);
        if self.done > 0 {
            ui.label(egui::RichText::new(i18n::tr_fmt("done for now - {count} reviewed", &[("count", &self.done.to_string())])).size(15.0));
        } else {
            ui.label(egui::RichText::new(tl!("nothing is due in this deck")).weak());
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if !self.seen.is_empty()
                && ui.button(i18n::tr_fmt("Repeat the {count} of this session", &[("count", &self.seen.len().to_string())])).clicked()
            {
                self.repeat();
            }
            if ui.button(tl!("Review a week ahead")).on_hover_text(tl!("Study before the schedule asks; nothing is graded")).clicked() {
                self.ahead(state);
            }
        });
    }

    /// Draws the statistics panel.
    pub fn stats_ui(&mut self, ui: &mut egui::Ui, state: &mut VaultState) {
        let report = state.srs_report();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Review statistics")).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("x").clicked() {
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
                ui.label(egui::RichText::new(tl!("Notes with the most cards")).strong());
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
