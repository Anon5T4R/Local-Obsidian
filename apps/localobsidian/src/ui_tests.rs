//! Headless UI tests of the F3 panels, on `egui_kittest`.
//!
//! The assertions walk the accessibility tree (what a screen reader and a test both see), so no GPU
//! is needed. With `LOCALOBSIDIAN_QA_RENDER=1` each test also renders the panel to
//! `target/qa-f3/<name>.png`, which is what the visual QA looks at - the render needs a GPU, so it
//! stays out of the default run. Rendering several screens in parallel (Vulkan on this machine)
//! trips an access violation, so the QA run is `-- --test-threads=1`; the default run never renders
//! and is safe to parallelize.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::panels::{self, PanelAction};
use crate::vault_state::VaultState;

/// A vault on disk, removed when the guard drops.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str, notes: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!("localobsidian-ui-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch dir");
        for (relative, text) in notes {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("parent");
            }
            std::fs::write(path, text).expect("write note");
        }
        Self(root)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A vault with something to find, to outline and to link.
fn demo_vault(tag: &str) -> (Scratch, VaultState) {
    let scratch = Scratch::new(
        tag,
        &[
            (
                "Sepse.md",
                "---\ntags: [uti]\n---\n\n# Sepse\n\nLactato na admissão.\nRepetir lactato em 2h.\n\n## Conduta\n\nAntibiótico em 1h. Ver [[Protocolo#Choque]].\n",
            ),
            ("Protocolos/Protocolo.md", "# Protocolo\n\n## Choque\n\nVolume e vasopressor.\n"),
            ("Sepse neonatal.md", "# Sepse neonatal\n\nDoses por peso.\n"),
        ],
    );
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    (scratch, state)
}

/// A harness whose panel hands its action to the test.
fn harness(state: VaultState) -> (Harness<'static, VaultState>, Rc<RefCell<PanelAction>>) {
    let action = Rc::new(RefCell::new(PanelAction::None));
    let sink = action.clone();
    let harness = Harness::builder()
        .with_size(egui::vec2(900.0, 620.0))
        .build_ui_state(move |ui, state: &mut VaultState| record(&sink, panels::search(ui, state)), state);
    (harness, action)
}

/// Keeps the last action a panel reported: the harness runs several frames, and only the frame that
/// handled the click carries it.
fn record(sink: &Rc<RefCell<PanelAction>>, action: PanelAction) {
    if action != PanelAction::None {
        *sink.borrow_mut() = action;
    }
}

/// Saves the harness's screen when the QA render is asked for.
fn shot<State>(harness: &mut Harness<'_, State>, name: &str) {
    if std::env::var_os("LOCALOBSIDIAN_QA_RENDER").is_none() {
        return;
    }
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/qa-f3");
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    if let Ok(image) = harness.render() {
        let _ = image.save(directory.join(format!("{name}.png")));
    }
}

#[test]
fn the_search_panel_lists_the_matching_lines_and_opens_the_note_from_one() {
    let (_scratch, mut state) = demo_vault("search");
    state.search.open = true;
    state.search.query = "lactato".to_string();
    state.run_search_now();

    let (mut harness, action) = harness(state);
    harness.run_steps(3);
    shot(&mut harness, "ui-search");

    // The note is a result, and both of its matching lines are listed.
    harness.get_by_label("Sepse");
    harness.get_by_label("Lactato na admissão.");
    harness.get_by_label("Repetir lactato em 2h.");

    // Clicking a line opens the note at that line.
    harness.get_by_label("Repetir lactato em 2h.").click();
    harness.run_steps(2);
    let action = action.borrow().clone();
    match action {
        PanelAction::Open { line, .. } => assert_eq!(line, Some(8), "the second match is on line 8 of the file"),
        other => panic!("a line should open the note at its line, got {other:?}"),
    }
}

#[test]
fn the_outline_panel_lists_the_headings_in_order() {
    let (scratch, mut state) = demo_vault("outline");
    state.open_note(&scratch.0.join("Sepse.md"));

    let action = Rc::new(RefCell::new(PanelAction::None));
    let sink = action.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(400.0, 620.0))
        .build_ui_state(move |ui, state: &mut VaultState| record(&sink, panels::outline(ui, state)), state);
    harness.run_steps(3);
    shot(&mut harness, "ui-outline");

    harness.get_by_label("Outline");
    let sepse = harness.get_by_label("Sepse");
    let conduta = harness.get_by_label("Conduta");
    assert!(sepse.rect().top() < conduta.rect().top(), "the outline keeps the document order");
    conduta.click();
    harness.run_steps(2);
    assert_eq!(*action.borrow(), PanelAction::GoToSection("Conduta".to_string()));
}

#[test]
fn the_backlinks_panel_lists_the_notes_that_link_the_open_one() {
    let (scratch, mut state) = demo_vault("backlinks");
    state.open_note(&scratch.0.join("Protocolos/Protocolo.md"));
    assert_eq!(state.backlinks_of_active(), ["Sepse"], "a section link is still a mention");

    let opened: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
    let sink = opened.clone();
    let mut harness = Harness::builder().with_size(egui::vec2(400.0, 400.0)).build_ui_state(
        move |ui, state: &mut VaultState| {
            let opened = panels::backlinks(ui, state);
            if opened.is_some() {
                *sink.borrow_mut() = opened;
            }
        },
        state,
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-backlinks");

    harness.get_by_label("Backlinks (1)");
    harness.get_by_label("Sepse").click();
    harness.run_steps(2);
    assert_eq!(opened.borrow().clone(), Some(scratch.0.join("Sepse.md")), "clicking a backlink opens that note");
}

#[test]
fn the_review_panel_shows_a_due_card_reveals_it_and_grades_it() {
    let scratch = Scratch::new("review", &[("Sepse.md", "# Sepse\n\n> [!card]- Qual a tríade do qSOFA?\n> FR, PAS e Glasgow.\n")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");

    // Opening the vault schedules the card of the note: due today, so the session has work.
    assert_eq!(state.srs_stats().total, 1, "{:?}", state.srs);
    let mut review = crate::review_ui::Review::default();
    review.start(&state, crate::review_ui::Deck::All);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(560.0, 900.0))
        .build_ui_state(|ui, (state, review): &mut (VaultState, crate::review_ui::Review)| review.ui(ui, state), (state, review));
    harness.run_steps(3);
    shot(&mut harness, "ui-review");

    harness.get_by_label("Qual a tríade do qSOFA?");
    harness.get_by_label_contains("Reveal").click();
    harness.run_steps(2);
    harness.get_by_label("FR, PAS e Glasgow.");
    shot(&mut harness, "ui-review-answer");

    harness.get_by_label("Good (3)").click();
    harness.run_steps(2);

    // The card left the session and the schedule on disk says it was answered once.
    let saved = std::fs::read_to_string(scratch.0.join(".openobsidian/srs.json")).expect("the schedule was written");
    assert!(saved.contains("\"reps\": 1"), "{saved}");
    assert!(saved.contains("\"due\":"), "{saved}");
    assert_eq!(harness.state().0.srs_stats().due, 0, "it is no longer due today");
}

#[test]
fn the_tag_chips_filter_the_tree() {
    let (_scratch, state) = demo_vault("tags");
    let mut harness =
        Harness::builder().with_size(egui::vec2(400.0, 300.0)).build_ui_state(|ui, state: &mut VaultState| panels::tags(ui, state), state);
    harness.run_steps(3);
    shot(&mut harness, "ui-tags");

    harness.get_by_label("uti 1").click();
    harness.run_steps(2);
    assert_eq!(harness.state().tag_filter.as_deref(), Some("uti"), "the chip filters the tree by that tag");
}

#[test]
fn the_graph_draws_the_vault_and_says_so_when_there_is_nothing_to_draw() {
    let (_scratch, state) = demo_vault("graph");
    let mut harness = Harness::builder().with_size(egui::vec2(640.0, 520.0)).build_ui_state(
        |ui, state: &mut VaultState| {
            crate::graph::show(ui, state);
        },
        state,
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-graph");

    // An empty vault draws the message instead of an empty canvas.
    let mut empty = Harness::builder().with_size(egui::vec2(400.0, 300.0)).build_ui_state(
        |ui, state: &mut VaultState| {
            crate::graph::show(ui, state);
        },
        VaultState::default(),
    );
    empty.run_steps(2);
    empty.get_by_label("there are no notes to draw");
}

#[test]
fn the_calendar_walks_the_months() {
    let state = (crate::calendar::today_iso(), vec![crate::calendar::today_iso()]);
    let mut harness = Harness::builder().with_size(egui::vec2(340.0, 380.0)).build_ui_state(
        |ui, (month, marked): &mut (String, Vec<String>)| {
            let days = marked.clone();
            crate::calendar::show(ui, month, &|iso: &str| days.iter().any(|day| day == iso), "en");
        },
        state,
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-calendar");

    let before = harness.state().0.clone();
    harness.get_by_label(">").click();
    harness.run_steps(2);
    assert_eq!(harness.state().0, crate::calendar::add_months(&before, 1), "the arrow moves one month ahead");
}
