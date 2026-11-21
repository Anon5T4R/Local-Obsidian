//! Headless UI tests of the F3 panels, on `egui_kittest`.
//!
//! The assertions walk the accessibility tree (what a screen reader and a test both see), so no GPU
//! is needed. With `LOCALOBSIDIAN_QA_RENDER=1` each test also renders the panel to
//! `target/qa-f3/<name>.png`, which is what the visual QA looks at - the render needs a GPU, so it
//! stays out of the default run. Rendering several screens in parallel (Vulkan on this machine)
//! trips an access violation, so the QA run is `-- --test-threads=1`; the default run never renders
//! and is safe to parallelize.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

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

/// The `+N more` toggle: what `app.rs` does when a query block asks for the rest (the test needs
/// the same behavior without a whole window).
fn toggle_query(state: &mut VaultState, source: String) {
    if !state.query_expanded.remove(&source) {
        state.query_expanded.insert(source);
    }
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
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/qa-f7");
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
            let opened = panels::backlinks(ui, state, 220.0);
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

    let mut harness = Harness::builder().with_size(egui::vec2(560.0, 900.0)).build_ui_state(
        |ui, (state, review, images): &mut (VaultState, crate::review_ui::Review, crate::images::Images)| review.ui(ui, state, images),
        (state, review, crate::images::Images::default()),
    );
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
    let (_scratch, mut state) = demo_vault("tags");
    // The section starts folded; the chips are one click away.
    state.tags_open = true;
    let mut harness = Harness::builder().with_size(egui::vec2(400.0, 500.0)).build_ui_state(
        |ui, state: &mut VaultState| {
            let _ = panels::tags(ui, state, 200.0);
        },
        state,
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-tags");

    harness.get_by_label("uti 1").click();
    harness.run_steps(2);
    assert_eq!(harness.state().tag_filter.as_deref(), Some("uti"), "the chip filters the tree by that tag");
}

/// The QA battery's note: the same file the vault generator writes, so the test's spec is the real
/// content and not a second demo.
const KITCHEN_SINK: &str = include_str!("../../../qa/kitchen-sink.md");

/// The window sizes the app is used at, wide to narrow: a table that fits 1920 can collapse at 800.
const QA_SIZES: [(f32, f32); 4] = [(1920.0, 1080.0), (1366.0, 768.0), (1024.0, 600.0), (800.0, 500.0)];

/// The QA battery: the kitchen-sink note laid out at every window size, with the assertions the
/// collapsed table and the overlapping sidebar would have tripped. This is the test that runs before
/// a phase closes; the PNGs are what the visual QA looks at.
#[test]
fn the_kitchen_sink_lays_out_at_every_window_size() {
    for (width, height) in QA_SIZES {
        let scratch = Scratch::new(&format!("battery-{}", width as u32), &[("Tudo.md", KITCHEN_SINK)]);
        let mut state = VaultState::default();
        state.open(scratch.0.clone()).expect("open the vault");
        state.open_note(&scratch.0.join("Tudo.md"));
        state.view = crate::vault_state::ViewMode::Read;
        let images = crate::images::Images::default();
        let mut harness = Harness::builder().with_size(egui::vec2(width, height)).build_ui_state(
            |ui, (state, images): &mut (VaultState, crate::images::Images)| {
                crate::editor::show(ui, state, images);
            },
            (state, images),
        );
        harness.run_steps(3);
        shot(&mut harness, &format!("battery-{}x{}", width as u32, height as u32));

        // The table shares the page instead of collapsing into a sliver: the header cells are on one
        // row, each column starts after the one before it, and the fourth column sits well into the
        // width (in the collapsed version all four fit inside the first ~200 px).
        let first = harness.get_by_label("Coluna curta").rect();
        let second = harness.get_by_label("Uma coluna com nome bem mais longo").rect();
        let fourth = harness.get_by_label("Outra").rect();
        assert!((first.top() - second.top()).abs() < 2.0, "{width}x{height}: header cells on different rows");
        assert!(second.left() > first.right() + 8.0, "{width}x{height}: the columns touch: {first:?} vs {second:?}");
        assert!(fourth.left() > width * 0.25, "{width}x{height}: the table collapsed to {} px", fourth.left());
        let cell = harness.get_by_label("texto médio na célula").rect();
        assert!(cell.height() < 60.0, "{width}x{height}: the cell became a tower ({:?})", cell.height());

        // Two blocks in a row keep the rhythm of the page (the query boxes used to touch each
        // other): the two `query` cards of the kitchen sink are a block pair like any other.
        let mut cards: Vec<egui::Rect> = harness.get_all_by_label("query").map(|node| node.rect()).collect();
        cards.sort_by(|a, b| a.top().total_cmp(&b.top()));
        assert!(cards.len() >= 2, "{width}x{height}: the two query cards are not drawn");
        let gap = cards[1].top() - cards[0].bottom();
        assert!(gap >= 12.0, "{width}x{height}: two blocks touch ({gap:.0} px of gap)");

        // `%%comentário%%` is a note to the author: it stays in the file and never reaches the page.
        assert!(harness.query_by_label_contains("um comentário do autor").is_none(), "{width}x{height}: the author's comment is on the page");

        // No two labels share a rectangle: that is what "two lists drawn in the same place" looks
        // like in the tree, and it is the cheapest way to catch it on any screen.
        let labels: Vec<egui::Rect> = harness.get_all_by_role(egui::accesskit::Role::Label).map(|node| node.rect()).collect();
        for (index, one) in labels.iter().enumerate() {
            for other in labels.iter().skip(index + 1) {
                let overlap = one.intersect(*other);
                if overlap.width() <= 2.0 || overlap.height() <= 2.0 {
                    continue;
                }
                // A label inside another is nesting (a run inside a line); two labels crossing each
                // other is the bug - that is what two lists drawn in the same place look like.
                let contains = |outer: &egui::Rect, inner: &egui::Rect| outer.contains(inner.min) && outer.contains(inner.max);
                if contains(one, other) || contains(other, one) {
                    continue;
                }
                panic!("{width}x{height}: two labels overlap: {one:?} and {other:?}");
            }
        }
    }
}

/// The window the shots are taken from: the app itself, not a panel, and the vault the user has when
/// it exists. `cargo xtask qa-shots` runs it; it is slower than two minutes, so it stays out of CI.
#[test]
#[ignore = "needs a built app and the QA vault: run `cargo xtask qa-vault && cargo xtask qa-shots`"]
fn the_window_shots_are_taken_from_the_generated_vault() {
    // The real work lives in xtask, which owns the process; this test only documents where it is.
    let shots = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/qa-f7");
    assert!(shots.is_dir(), "run `cargo xtask qa-shots`; it writes into {}", shots.display());
}

#[test]
fn a_wikilink_in_the_reading_view_is_clickable() {
    let scratch = Scratch::new("wikilink-click", &[("Origem.md", "# Origem\n\nVeja [[Alvo]] para o resto.\n"), ("Alvo.md", "# Alvo\n")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Origem.md"));
    state.view = crate::vault_state::ViewMode::Read;
    let images = crate::images::Images::default();
    let action = Rc::new(RefCell::new(crate::editor::EditorAction::None));
    let sink = action.clone();
    let mut harness = Harness::builder().with_size(egui::vec2(560.0, 320.0)).build_ui_state(
        move |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            let done = crate::editor::show(ui, state, images);
            if done != crate::editor::EditorAction::None {
                *sink.borrow_mut() = done;
            }
        },
        (state, images),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-wikilink");

    harness.get_by_label("Alvo").click();
    harness.run_steps(2);
    assert_eq!(*action.borrow(), crate::editor::EditorAction::Navigate("Alvo".into()), "the link navigates");
}

#[test]
fn the_reading_view_draws_no_emoji_and_keeps_the_arrows() {
    // Emoji are never drawn (no emoji font ships); arrows and maths signs are text and keep being
    // drawn when the fonts have the glyph.
    assert!(crate::editor::is_emoji('\u{1F3E5}') && crate::editor::is_emoji('\u{1F9ED}') && crate::editor::is_emoji('\u{FE0F}'));
    assert!(!crate::editor::is_emoji('\u{2192}') && crate::editor::is_symbol('\u{2192}'), "an arrow is text, not an emoji");
    assert!(crate::editor::is_symbol('\u{2264}'), "maths operators are symbols");
    assert!(!crate::editor::is_symbol('\u{00A7}'), "a section sign is text");
    assert!(!crate::editor::is_emoji('\u{00E3}') && !crate::editor::is_symbol('\u{00E3}'), "letters pass through");
}
#[test]
fn a_document_in_the_vault_is_listed_and_clicked_into_the_system_handler() {
    // The generated vault holds a `.txt`, a `.docx` and a `.pdf` next to the notes. They are not
    // notes - the app does not edit them - but they are files of the vault, and dropping them made
    // the tree look emptier than the folder. They are listed and handed to the desktop on click.
    let scratch = Scratch::new("binaries", &[("Nota.md", "# Nota\n"), ("leia-me.txt", "texto"), ("protocolo.pdf", "%PDF-1.4"), ("anexo.docx", "PK")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    assert_eq!(state.files.len(), 1, "only the note is a note: {:?}", state.files.iter().map(|note| &note.name).collect::<Vec<_>>());
    let action = Rc::new(RefCell::new(crate::sidebar::SidebarAction::None));
    let sink = action.clone();
    let mut harness = Harness::builder().with_size(egui::vec2(360.0, 320.0)).build_ui_state(
        move |ui, state: &mut VaultState| {
            let done = crate::sidebar::show(ui, state);
            if done != crate::sidebar::SidebarAction::None {
                *sink.borrow_mut() = done;
            }
        },
        state,
    );
    harness.run_steps(2);
    shot(&mut harness, "ui-binaries");

    // The three documents are listed, with their extensions; the note keeps its bare name.
    harness.get_by_label("protocolo.pdf");
    harness.get_by_label("leia-me.txt");
    harness.get_by_label("anexo.docx");
    harness.get_by_label("Nota");

    harness.get_by_label("protocolo.pdf").click();
    harness.run_steps(2);
    assert_eq!(
        *action.borrow(),
        crate::sidebar::SidebarAction::OpenBinary(scratch.0.join("protocolo.pdf")),
        "a click hands the document to the desktop"
    );
}

#[test]
fn the_search_panel_is_a_sidebar_and_never_the_window() {
    // A text field asking for `f32::INFINITY` made the panel grow to the whole window: the input
    // stretched to the edge, the results column was squeezed and the note disappeared under it.
    let (_scratch, mut state) = demo_vault("search-width");
    state.search.open = true;
    state.search.query = "lactato".to_string();
    state.run_search_now();

    let width = Rc::new(Cell::new(0.0f32));
    let sink = width.clone();
    let mut harness = Harness::builder().with_size(egui::vec2(1920.0, 1080.0)).build_ui_state(
        move |ui, state: &mut VaultState| {
            let panel = egui::Panel::right("search").resizable(true).default_size(320.0).show(ui, |ui| {
                let _ = panels::search(ui, state);
            });
            sink.set(panel.response.rect.width());
        },
        state,
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-search-panel-1920");

    let measured = width.get();
    assert!(measured <= 520.0, "the search panel is a sidebar, not the window: {measured:.0} px");
    assert!(measured >= 240.0, "and it is wide enough to read a result: {measured:.0} px");
}

#[test]
fn the_reading_column_is_centred_in_its_pane() {
    // The measure is what keeps the text readable; the margin on its left has to be the margin on
    // its right, or the column reads as if it were glued to the sidebar.
    let paragraph = "Parágrafo largo de teste que precisa quebrar em várias linhas para ocupar a coluna inteira, \
                     porque só um texto que enche a medida mostra onde a medida começa e onde ela termina.";
    let scratch = Scratch::new("reading-column", &[("Largo.md", &format!("# Título\n\n{paragraph}\n"))]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Largo.md"));
    state.view = crate::vault_state::ViewMode::Read;
    let pane = Rc::new(Cell::new(egui::Rect::NOTHING));
    let pane_sink = pane.clone();
    let mut harness = Harness::builder().with_size(egui::vec2(1920.0, 1080.0)).build_ui_state(
        move |ui, state: &mut VaultState| {
            pane_sink.set(ui.max_rect());
            let mut images = crate::images::Images::default();
            let _ = crate::editor::show(ui, state, &mut images);
        },
        state,
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-reading-column-1920");

    let column = harness.get_by_label_contains("Parágrafo largo").rect();
    let pane = pane.get();
    let left = column.left() - pane.left();
    let right = pane.right() - column.right();
    assert!(left >= 380.0, "the column has a left margin: {left:.0} px");
    assert!(right >= 380.0, "and a right one: {right:.0} px");
    assert!((left - right).abs() < 40.0, "the two margins are the same: left {left:.0}, right {right:.0}");
    assert!(column.width() <= 900.0, "the measure is a column, not the window: {:.0} px", column.width());
}

#[test]
fn blocks_of_the_reading_view_keep_the_rhythm_between_them() {
    // Two consecutive query boxes looked glued: the gap between blocks is what keeps a note
    // readable, and a fence is a block like any other.
    let note = "# T\n\n```rust\na\n```\n\n```rust\nb\n```\n";
    let scratch = Scratch::new("block-rhythm", &[("Ritmo.md", note)]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Ritmo.md"));
    state.view = crate::vault_state::ViewMode::Read;
    let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(
        |ui, state: &mut VaultState| {
            let mut images = crate::images::Images::default();
            let _ = crate::editor::show(ui, state, &mut images);
        },
        state,
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-block-rhythm");

    let first = harness.get_by_label("a").rect();
    let second = harness.get_by_label("b").rect();
    let gap = second.top() - first.bottom();
    assert!(gap >= 30.0, "the two blocks keep a gap of their own: {gap:.0} px");
    assert!(gap <= 64.0, "and it is a rhythm, not a hole: {gap:.0} px");
}

#[test]
fn the_reading_column_fills_the_pane_when_the_pane_is_narrow() {
    // In Split the pane is narrower than the measure: the text uses the pane instead of leaving a
    // dead margin on the right, which is what makes a split look misaligned.
    let paragraph = "Parágrafo largo de teste que precisa quebrar em várias linhas para ocupar a coluna inteira, \
                     porque só um texto que enche a medida mostra onde a medida começa e onde ela termina.";
    let scratch = Scratch::new("split-column", &[("Largo.md", &format!("# Título\n\n{paragraph}\n"))]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Largo.md"));
    state.view = crate::vault_state::ViewMode::Split;
    let pane = Rc::new(Cell::new(egui::Rect::NOTHING));
    let pane_sink = pane.clone();
    let mut harness = Harness::builder().with_size(egui::vec2(1366.0, 768.0)).build_ui_state(
        move |ui, state: &mut VaultState| {
            pane_sink.set(ui.max_rect());
            let mut images = crate::images::Images::default();
            let _ = crate::editor::show(ui, state, &mut images);
        },
        state,
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-split-column-1366");

    let column = harness.get_by_label_contains("Parágrafo largo").rect();
    let pane = pane.get();
    let right = pane.right() - column.right();
    assert!(right <= 60.0, "the text reaches the edge of its pane: {right:.0} px of dead margin");
    assert!(column.width() >= 300.0, "the pane is used: {:.0} px", column.width());
}

#[test]
fn the_ai_connection_is_a_centred_dialog_of_measured_width() {
    // The dialog used to take the window: the fields asked for `f32::INFINITY` and the window grew
    // to the screen - inputs 1900 px long, `Save` on the left, `x` a screen away, the note covered.
    for (width, height) in [(1920.0f32, 1080.0f32), (800.0, 600.0)] {
        let mut chat = crate::ai_ui::Chat::default();
        chat.settings_open = true;
        let dialog = Rc::new(Cell::new(egui::Rect::NOTHING));
        let dialog_sink = dialog.clone();
        let pane = Rc::new(Cell::new(egui::Rect::NOTHING));
        let pane_sink = pane.clone();
        let mut harness = Harness::builder().with_size(egui::vec2(width, height)).build_ui_state(
            move |ui, chat: &mut crate::ai_ui::Chat| {
                pane_sink.set(ui.max_rect());
                if let Some(rect) = chat.settings(ui) {
                    dialog_sink.set(rect);
                }
            },
            chat,
        );
        harness.run_steps(3);
        shot(&mut harness, &format!("ui-ai-dialog-{}x{}", width as u32, height as u32));

        let rect = dialog.get();
        assert!(rect.width() <= 560.0, "the dialog is a dialog, not the window: {:.0} px at {width:.0}", rect.width());
        assert!(rect.width() >= 460.0, "and it is wide enough for a base URL: {:.0} px", rect.width());
        assert!(rect.right() <= width, "it fits the window: right edge {:.0} of {width:.0}", rect.right());
        let off_centre = (rect.center().x - pane.get().center().x).abs();
        assert!(off_centre < 24.0, "it is centred: {off_centre:.0} px off at {width:.0}");
    }
}

#[test]
fn the_side_panels_keep_their_width_at_every_window_size() {
    // The bug family: a panel grows to fit its content, so one `f32::INFINITY` inside it turned a
    // sidebar into the page. Each panel is drawn here exactly as `app.rs::page` draws it - same
    // default, same share, same absolute ceiling.
    for (width, height) in [(1920.0f32, 1080.0f32), (800.0, 600.0)] {
        for (name, left, default, share, absolute) in [
            ("sidebar", true, 240.0f32, 0.34f32, 380.0f32),
            ("search", false, 320.0, 0.5, 520.0),
            ("outline", false, 220.0, 0.5, 400.0),
            ("ai-chat", false, 340.0, 0.5, 560.0),
            ("review", false, 320.0, 0.5, 520.0),
            ("review-stats", false, 300.0, 0.5, 460.0),
        ] {
            let (scratch, mut state) = demo_vault(&format!("panel-{name}-{}", width as u32));
            state.open_note(&scratch.0.join("Sepse.md"));
            state.search.open = true;
            state.search.query = "lactato".to_string();
            state.run_search_now();
            state.outline_open = true;
            let mut chat = crate::ai_ui::Chat::default();
            chat.open = true;
            let mut review = crate::review_ui::Review::default();
            review.open = true;
            review.stats_open = true;

            let measured = Rc::new(Cell::new(0.0f32));
            let sink = measured.clone();
            let mut harness = Harness::builder().with_size(egui::vec2(width, height)).build_ui_state(
                move |ui, (state, chat, review): &mut (VaultState, crate::ai_ui::Chat, crate::review_ui::Review)| {
                    let ceiling = (ui.available_width() * share).clamp(160.0, absolute);
                    let panel = if left { egui::Panel::left(name) } else { egui::Panel::right(name) };
                    let drawn = panel.resizable(true).default_size(default).max_size(ceiling).show(ui, |ui| match name {
                        "sidebar" => {
                            let _ = crate::sidebar::show(ui, state);
                        }
                        "search" => {
                            let _ = panels::search(ui, state);
                        }
                        "outline" => {
                            let _ = panels::outline(ui, state);
                        }
                        "ai-chat" => chat.ui(ui, state),
                        "review" => review.ui(ui, state, &mut crate::images::Images::default()),
                        _ => review.stats_ui(ui, state),
                    });
                    sink.set(drawn.response.rect.width());
                },
                (state, chat, review),
            );
            harness.run_steps(3);

            let measured = measured.get();
            assert!(measured <= absolute + 1.0, "{name} at {width:.0}: {measured:.0} px is more than its ceiling of {absolute:.0}");
            assert!(measured >= 160.0, "{name} at {width:.0}: {measured:.0} px is too narrow to use");
            assert!(measured < width * 0.6, "{name} at {width:.0}: {measured:.0} px left no room for the note");
        }
    }
}

#[test]
fn the_arrows_walk_the_notes_that_were_visited() {
    // Three notes visited from the tree, then two steps back and one forward: the note in the
    // header has to be the one the history says, and the arrows are the ones the mouse and the
    // keyboard drive.
    let (scratch, mut state) = demo_vault("history");
    state.open_note(&scratch.0.join("Sepse.md"));
    state.open_note(&scratch.0.join("Sepse neonatal.md"));
    state.open_note(&scratch.0.join("Protocolos/Protocolo.md"));
    let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(
        |ui, state: &mut VaultState| {
            let mut images = crate::images::Images::default();
            match crate::editor::show(ui, state, &mut images) {
                crate::editor::EditorAction::Back => {
                    assert!(state.go_back(), "the arrow is only live when there is a note behind");
                }
                crate::editor::EditorAction::Forward => {
                    assert!(state.go_forward(), "and the same ahead");
                }
                _ => {}
            }
        },
        state,
    );
    harness.run_steps(2);
    shot(&mut harness, "ui-history");

    let note = |harness: &Harness<'_, VaultState>| {
        harness.state().active.as_ref().and_then(|path| path.file_stem()).map(|stem| stem.to_string_lossy().to_string())
    };
    assert_eq!(note(&harness).as_deref(), Some("Protocolo"), "the last note visited");

    harness.get_by_label("Back").click();
    harness.run_steps(2);
    assert_eq!(note(&harness).as_deref(), Some("Sepse neonatal"), "one step back");

    harness.get_by_label("Back").click();
    harness.run_steps(2);
    assert_eq!(note(&harness).as_deref(), Some("Sepse"), "two steps back");

    harness.get_by_label("Forward").click();
    harness.run_steps(2);
    assert_eq!(note(&harness).as_deref(), Some("Sepse neonatal"), "and forward again");
}

#[test]
fn a_note_deck_reviews_only_that_notes_cards() {
    // The report: "I clicked Review this note and got the whole vault". The wiring passes
    // `Deck::Note(<relative path>)`; this is the test that holds it.
    let scratch = Scratch::new(
        "review-scope",
        &[
            ("MOC.md", "# MOC\n\n> [!card] Pergunta do MOC\n> Resposta do MOC\n"),
            ("Outra.md", "# Outra\n\n> [!card] Pergunta da outra\n> Resposta da outra\n\n> [!card] Segunda\n> Resposta\n"),
            ("Vazia.md", "# Vazia\n\nsem cards\n"),
        ],
    );
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    assert_eq!(state.srs.cards.len(), 3, "the schedule was built from the notes");

    let mut review = crate::review_ui::Review::default();
    review.start(&state, crate::review_ui::Deck::Note("MOC.md".to_string()));
    assert_eq!(review.queue_len(), 1, "only the open note's card");
    assert_eq!(review.scope_cards(), 1, "and the header counts the deck, not the vault");

    review.start(&state, crate::review_ui::Deck::Note("Outra.md".to_string()));
    assert_eq!(review.queue_len(), 2, "the other note has two");

    review.start(&state, crate::review_ui::Deck::All);
    assert_eq!(review.queue_len(), 3, "the vault has all three");
}

#[test]
fn the_review_header_names_its_deck_and_its_numbers() {
    let scratch = Scratch::new(
        "review-header",
        &[
            (
                "MOC - Medicina.md",
                "# Moc\n\n> [!card] A **tetralogia de Fallot** e a `CIA` são o quê?\n> Erros de septação; o ==destaque== também renderiza.\n",
            ),
            ("Outra.md", "# Outra\n\nsem cards\n"),
        ],
    );
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("MOC - Medicina.md"));
    let mut review = crate::review_ui::Review::default();
    review.start(&state, crate::review_ui::Deck::Note("MOC - Medicina.md".to_string()));
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(560.0, 420.0)).build_ui_state(
        |ui, (state, review, _images): &mut (VaultState, crate::review_ui::Review, crate::images::Images)| {
            review.ui(ui, state, _images);
        },
        (state, review, images),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-review-header");

    // The scope is the note's own name (in the deck selector), and the counts belong to that deck.
    harness.get_by_label("Deck");
    assert_eq!(harness.state().1.deck, crate::review_ui::Deck::Note("MOC - Medicina.md".to_string()));
    harness.get_by_label("1 card in this deck · 1 due today · 0 done in this session");
    harness.get_by_label_contains("1 due today");
    harness.get_by_label_contains("0 done in this session");
}

#[test]
fn a_note_without_cards_says_so_and_shows_the_syntax() {
    let scratch = Scratch::new("review-empty", &[("Vazia.md", "# Vazia\n\nsem cards\n")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    let mut review = crate::review_ui::Review::default();
    review.start(&state, crate::review_ui::Deck::Note("Vazia.md".to_string()));
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(460.0, 420.0)).build_ui_state(
        |ui, (state, review, images): &mut (VaultState, crate::review_ui::Review, crate::images::Images)| {
            review.ui(ui, state, images);
        },
        (state, review, images),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-review-empty");

    harness.get_by_label_contains("Vazia has no cards yet");
    harness.get_by_label_contains("[!card] the question");
    harness.get_by_label_contains("the answer, under it");
}

#[test]
fn practice_never_touches_the_schedule() {
    // A deck whose cards are all far in the future: the panel offers practice, and grading there
    // must not write to `srs.json` - practice is not review.
    let scratch = Scratch::new("review-practice", &[("Nota.md", "# Nota\n\n> [!card] Pergunta\n> Resposta\n")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    for card in state.srs.cards.values_mut() {
        card.due = "2099-01-01".to_string();
    }
    let mut review = crate::review_ui::Review::default();
    review.start(&state, crate::review_ui::Deck::All);
    assert_eq!(review.queue_len(), 0, "nothing is due today");

    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(460.0, 460.0)).build_ui_state(
        |ui, (state, review, images): &mut (VaultState, crate::review_ui::Review, crate::images::Images)| {
            review.ui(ui, state, images);
        },
        (state, review, images),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-review-practice");

    harness.get_by_label_contains("1 card in");
    harness.get_by_label_contains("Practice the 1 card").click();
    harness.run_steps(2);
    harness.get_by_label_contains("practice - nothing is graded");
    harness.get_by_label("Reveal (Space)").click();
    harness.run_steps(2);
    harness.get_by_label("Good (3)").click();
    harness.run_steps(2);

    // The card was answered, and the schedule never heard about it.
    let state = &harness.state().0;
    for card in state.srs.cards.values() {
        assert_eq!(card.reps, 0, "practice graded a card");
        assert_eq!(card.due, "2099-01-01", "practice moved a due date");
    }
}

#[test]
fn the_statistics_panel_follows_the_deck_of_the_session() {
    // The statistics were always the vault's, even in a one-note session: the panel's numbers have
    // to be the deck's.
    let scratch = Scratch::new(
        "review-stats",
        &[("MOC.md", "# Moc\n\n> [!card] Uma\n> Resposta\n"), ("Outra.md", "# Outra\n\n> [!card] Duas\n> Resposta\n\n> [!card] Três\n> Resposta\n")],
    );
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    assert_eq!(state.srs_report_for(None).stats.total, 3);
    assert_eq!(state.srs_report_for(Some(&["MOC.md".to_string()])).stats.total, 1, "the note's own cards");
    assert_eq!(state.deck_cards(Some(&["Outra.md".to_string()])), 2);

    let mut review = crate::review_ui::Review::default();
    review.start(&state, crate::review_ui::Deck::Note("MOC.md".to_string()));
    review.stats_open = true;
    let mut harness = Harness::builder().with_size(egui::vec2(420.0, 620.0)).build_ui_state(
        |ui, (state, review): &mut (VaultState, crate::review_ui::Review)| {
            review.stats_ui(ui, state);
        },
        (state, review),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-review-stats");

    // The panel names the deck it is reporting, right next to the title.
    harness.get_by_label("Review statistics");
    harness.get_by_label("MOC");
}

#[test]
fn the_deck_can_be_changed_from_the_panel() {
    // The panel used to leave the deck frozen at whatever the menu opened. The selector lists the
    // decks the vault has, with their counts, and switching reloads the session.
    let scratch = Scratch::new(
        "review-selector",
        &[
            ("MOC.md", "---\ntags: [uti]\n---\n\n# Moc\n\n> [!card] Do MOC\n> Resposta\n"),
            ("Outra.md", "---\ntags: [uti]\n---\n\n# Outra\n\n> [!card] Da outra\n> Resposta\n"),
        ],
    );
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("MOC.md"));
    let mut review = crate::review_ui::Review::default();
    review.start(&state, crate::review_ui::Deck::Note("MOC.md".to_string()));
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(560.0, 420.0)).build_ui_state(
        |ui, (state, review, images): &mut (VaultState, crate::review_ui::Review, crate::images::Images)| {
            review.ui(ui, state, images);
        },
        (state, review, images),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-review-selector");

    // The selector is closed: it shows the deck in use. Opening it lists the decks with counts.
    harness.get_by_label("Deck").click();
    harness.run_steps(2);
    harness.get_by_label_contains("the whole vault (2)");
    harness.get_by_label_contains("#uti (2)");

    harness.get_by_label_contains("the whole vault (2)").click();
    harness.run_steps(2);
    assert_eq!(harness.state().1.deck, crate::review_ui::Deck::All);
    assert_eq!(harness.state().1.queue_len(), 2, "the new deck's queue");
}

#[test]
fn the_statistics_panel_lists_the_next_cards_of_the_session() {
    let scratch = Scratch::new(
        "review-queue",
        &[(
            "MOC.md",
            "# Moc\n\n> [!card] Primeira pergunta\n> Resposta\n\n> [!card] Segunda pergunta\n> Resposta\n\n> [!card] Terceira pergunta\n> Resposta\n",
        )],
    );
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    let mut review = crate::review_ui::Review::default();
    review.start(&state, crate::review_ui::Deck::All);
    review.stats_open = true;
    let mut harness = Harness::builder().with_size(egui::vec2(460.0, 720.0)).build_ui_state(
        |ui, (state, review): &mut (VaultState, crate::review_ui::Review)| {
            review.stats_ui(ui, state);
        },
        (state, review),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-review-queue");

    harness.get_by_label("Next in this session");
    // The cards after the one on screen, each with the note it came from.
    assert!(harness.get_all_by_label("MOC").count() >= 2, "each row names its note");
    assert!(harness.query_by_label("Primeira pergunta").is_none(), "the current card is not in 'next'");
    harness.get_by_label("Segunda pergunta");
    harness.get_by_label("Terceira pergunta");
}

#[test]
fn a_query_block_answers_with_the_notes_it_selects() {
    // The "convert test" note: a fenced `query` block used to render as a static box. It is a
    // question, and the reading view answers it against the vault as it is now.
    let scratch = Scratch::new(
        "query-block",
        &[
            ("Q.md", "# Q\n\n```query\ntag: #tipo-patologia\nsort: titulo\n```\n"),
            ("Anafilaxia.md", "---\ntags: [tipo-patologia]\n---\n\n# Anafilaxia\n"),
            ("Asma.md", "---\ntags: [tipo-patologia]\n---\n\n# Asma\n"),
            ("Outra.md", "---\ntags: [tipo-indice]\n---\n\n# Outra\n"),
        ],
    );
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Q.md"));
    state.view = crate::vault_state::ViewMode::Read;
    let action = Rc::new(RefCell::new(crate::editor::EditorAction::None));
    let sink = action.clone();
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(760.0, 520.0)).build_ui_state(
        move |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            let done = crate::editor::show(ui, state, images);
            if done != crate::editor::EditorAction::None {
                *sink.borrow_mut() = done;
            }
        },
        (state, images),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-query-block");

    harness.get_by_label_contains("2 results");
    // The notes that carry the tag, in name order, and not the one that does not.
    harness.get_by_label("Anafilaxia");
    harness.get_by_label("Asma");
    assert!(harness.query_by_label("Outra").is_none(), "the other tag is not selected");

    harness.get_by_label("Asma").click();
    harness.run_steps(2);
    assert_eq!(*action.borrow(), crate::editor::EditorAction::Navigate("Asma".into()), "a result opens the note");
}

#[test]
fn a_query_block_says_what_it_did_not_understand() {
    let scratch =
        Scratch::new("query-bad", &[("Q.md", "# Q\n\n```query\ntag: #uti\nsort: alfabetico\n```\n"), ("A.md", "---\ntags: [uti]\n---\n\n# A\n")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Q.md"));
    state.view = crate::vault_state::ViewMode::Read;
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(620.0, 420.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            let _ = crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-query-bad");
    harness.get_by_label_contains("1 result");
    harness.get_by_label_contains("not understood: sort: alfabetico");
}

#[test]
fn the_vault_diagnostics_lists_what_is_wrong_and_opens_the_note() {
    let scratch = Scratch::new(
        "diagnostics",
        &[
            ("A.md", "# A\n\n[[Fantasma]] e [[B]]\n"),
            ("B.md", "# B\n\nvolta [[A]]\n"),
            ("So.md", "# So\n\nninguém liga\n"),
            ("Pasta/X.md", "# X\n\nnada\n"),
            ("Outra/X.md", "# X\n\nnada\n"),
        ],
    );
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    let opened: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
    let sink = opened.clone();
    let mut harness = Harness::builder().with_size(egui::vec2(640.0, 560.0)).build_ui_state(
        move |ui, state: &mut VaultState| {
            if let Some(path) = crate::diagnostics_ui::show(ui, state) {
                *sink.borrow_mut() = Some(path);
            }
        },
        state,
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-diagnostics");

    harness.get_by_label("1 broken link");
    harness.get_by_label("Fantasma");
    harness.get_by_label("A"); // the note that writes the broken link
    harness.get_by_label("So.md");
    harness.get_by_label("1 duplicated name");
    // The duplicate name lists both files (and each of them is also an orphan: two sections, on purpose).
    assert!(harness.get_all_by_label("Pasta/X.md").count() >= 1);
    assert!(harness.get_all_by_label("Outra/X.md").count() >= 1);

    harness.get_by_label("So.md").click();
    harness.run_steps(2);
    assert_eq!(*opened.borrow(), Some(scratch.0.join("So.md")));
}

#[test]
fn a_healthy_vault_says_there_is_nothing_to_fix() {
    let scratch = Scratch::new("diagnostics-clean", &[("A.md", "[[B]]"), ("B.md", "[[A]]")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    let mut harness = Harness::builder().with_size(egui::vec2(480.0, 320.0)).build_ui_state(
        |ui, state: &mut VaultState| {
            let _ = crate::diagnostics_ui::show(ui, state);
        },
        state,
    );
    harness.run_steps(2);
    harness.get_by_label_contains("nothing to fix");
}

#[test]
fn the_find_bar_walks_the_matches_of_the_note() {
    // Find in the note being read: the counter says how many, and the bar walks them.
    let scratch = Scratch::new(
        "find-in-note",
        &[("Nota.md", "# Sepse\n\nSepse é grave. Outra sepse aqui, e um [[Alvo]] no meio.\n"), ("Alvo.md", "# Alvo\n\nnada\n")],
    );
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Nota.md"));
    state.view = crate::vault_state::ViewMode::Read;
    state.find.open();
    state.find.query = "sepse".to_string();
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 620.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            let _ = crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-find");

    // The heading, the first word of the paragraph and the one after the comma: the bar counts them
    // and starts on the first.
    assert_eq!(harness.state().0.find.total, 3);
    assert_eq!(harness.state().0.find.current, 0);

    harness.get_by_label("Next match").click();
    harness.run_steps(2);
    assert_eq!(harness.state().0.find.current, 1);

    harness.get_by_label("Previous match").click();
    harness.run_steps(2);
    assert_eq!(harness.state().0.find.current, 0);
    harness.get_by_label("Previous match").click();
    harness.run_steps(2);
    assert_eq!(harness.state().0.find.current, 2, "before the first comes the last");
    assert!(harness.state().0.find.open, "the bar stays open until it is closed");
}

#[test]
fn a_note_with_no_match_says_zero_and_stays_open() {
    let scratch = Scratch::new("find-none", &[("Nota.md", "# Nota\n\nnada aqui\n")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Nota.md"));
    state.view = crate::vault_state::ViewMode::Read;
    state.find.open();
    state.find.query = "inexistente".to_string();
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(760.0, 480.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            let _ = crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(3);
    assert_eq!(harness.state().0.find.total, 0, "nothing matches");
    assert_eq!(harness.state().0.find.current, 0);
    // The bar is still there to be corrected, and closes on its own ×.
    harness.get_by_label("Close the find bar").click();
    harness.run_steps(2);
    assert!(!harness.state().0.find.open);
}

#[test]
fn ai_flashcards_write_a_new_note_beside_and_leave_the_original_alone() {
    // The action's contract: a new note next to the original, the original untouched, the cards in
    // the schedule, and one undo that removes only what the action created.
    let original = "---\ntipo: sistema\ntags: [uti]\n---\n\n# MOC\n\nCorpo da nota original.\n";
    let scratch = Scratch::new("flashcards", &[("MOC.md", original)]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("MOC.md"));

    let mut action = crate::flashcards::Flashcards::default();
    action.start(&state, None);
    action.apply(&mut state, r#"Aqui vão: {"cards":[{"question":"O que é sepse?","answer":"Resposta do modelo."}]}"#);

    let created = scratch.0.join("MOC AI.md");
    assert!(created.exists(), "the new note is beside the original");
    assert!(std::fs::read_to_string(&created).unwrap().contains("> [!card] O que é sepse?"));
    let text = std::fs::read_to_string(&created).unwrap();
    assert!(text.contains("> ==Resposta do modelo.=="), "the answer is the cloze term: {text}");
    assert!(text.contains("flashcards-ai"), "the generated note carries its tag: {text}");
    assert!(text.contains("tipo: sistema"), "the original's frontmatter comes along: {text}");

    // The original is byte for byte what it was before the action.
    assert_eq!(std::fs::read_to_string(scratch.0.join("MOC.md")).unwrap(), original);
    // And the card reached the schedule (the review panel will offer it).
    assert!(state.srs.cards.values().any(|card| card.q.contains("O que é sepse?")), "the card is in the schedule");

    action.undo(&mut state);
    assert!(!created.exists(), "undo removes the note the action created");
    assert_eq!(std::fs::read_to_string(scratch.0.join("MOC.md")).unwrap(), original, "and never the original");
}

#[test]
fn the_flashcards_dialog_names_the_note_it_will_create() {
    let scratch = Scratch::new("flashcards-dialog", &[("MOC - Medicina.md", "# MOC\n\ncorpo\n")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("MOC - Medicina.md"));
    let mut action = crate::flashcards::Flashcards::default();
    action.start(&state, None);
    let chat = crate::ai_ui::Chat::default();
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 620.0)).build_ui_state(
        |ui, (state, action, chat): &mut (VaultState, crate::flashcards::Flashcards, crate::ai_ui::Chat)| {
            action.dialog(ui, state, chat);
        },
        (state, action, chat),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-flashcards");

    // Destructividade zero, said before the action runs.
    harness.get_by_label_contains("MOC - Medicina AI");
    harness.get_by_label("Cards per section");
    harness.get_by_label("Create");
    harness.get_by_label("Cancel");
}

#[test]
fn a_query_block_expands_when_asked() {
    // Twenty notes match; the block shows a dozen and offers the rest, which is the one thing the
    // Electron cannot do with its static "N results".
    let mut notes: Vec<(String, String)> = vec![("Q.md".to_string(), "# Q\n\n```query\ntag: #big\n```\n".to_string())];
    for index in 0..20 {
        notes.push((format!("N{index:02}.md"), format!("---\ntags: [big]\n---\n\n# N{index:02}\n")));
    }
    let borrowed: Vec<(&str, &str)> = notes.iter().map(|(name, text)| (name.as_str(), text.as_str())).collect();
    let scratch = Scratch::new("query-expand", &borrowed);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Q.md"));
    state.view = crate::vault_state::ViewMode::Read;
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            if let crate::editor::EditorAction::ToggleQuery(source) = crate::editor::show(ui, state, images) {
                toggle_query(state, source);
            }
        },
        (state, images),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-query-expand");

    harness.get_by_label("20 results");
    harness.get_by_label("N00");
    assert!(harness.query_by_label("N19").is_none(), "the twelfth result is the last one shown");

    harness.get_by_label_contains("+8 more").click();
    harness.run_steps(2);
    harness.get_by_label("N19");

    harness.get_by_label("show fewer").click();
    harness.run_steps(2);
    assert!(harness.query_by_label("N19").is_none(), "folded back");
}

#[test]
fn a_link_to_a_note_is_a_link_and_one_that_points_at_nothing_is_not() {
    // The affordance the reader asked for: the resolvable link is the accent (and the hand cursor),
    // the one that names a note nobody wrote is underlined text that does not pretend to be a link.
    let scratch = Scratch::new("links", &[("Nota.md", "# Nota\n\n[[Alvo]] e [[Fantasma]] e ==destaque==\n"), ("Alvo.md", "# Alvo\n\nnada\n")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Nota.md"));
    state.view = crate::vault_state::ViewMode::Read;
    let action = Rc::new(RefCell::new(crate::editor::EditorAction::None));
    let sink = action.clone();
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(760.0, 420.0)).build_ui_state(
        move |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            let done = crate::editor::show(ui, state, images);
            if done != crate::editor::EditorAction::None {
                *sink.borrow_mut() = done;
            }
        },
        (state, images),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-links-dark");
    harness.ctx.set_visuals(egui::Visuals::light());
    harness.run_steps(2);
    shot(&mut harness, "ui-links-light");

    // The resolvable link navigates; the dangling one is not clickable at all.
    harness.get_by_label("Fantasma").click();
    harness.run_steps(2);
    assert_eq!(*action.borrow(), crate::editor::EditorAction::None, "a dangling link does nothing");
    harness.get_by_label("Alvo").click();
    harness.run_steps(2);
    assert_eq!(*action.borrow(), crate::editor::EditorAction::Navigate("Alvo".into()));
    // The highlight is still a highlight (the link work did not touch `==`).
    harness.get_by_label("destaque");
}

#[test]
fn the_chat_panel_shows_the_suite_panel() {
    // The panel is the suite's component now: header with the model, the roll, the composer and the
    // actions, drawn the same way in every app.
    let root = std::env::temp_dir().join(format!("localobsidian-chat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("scratch dir");
    std::fs::write(root.join("Nota.md"), "# Nota\n\ncorpo\n").expect("write");
    let mut state = VaultState::default();
    state.open(root.clone()).expect("open the vault");
    state.open_note(&root.join("Nota.md"));
    let mut chat = crate::ai_ui::Chat::default();
    chat.open = true;
    chat.panel_mut().push_answer("Resposta da IA sobre a nota.");
    let mut harness = Harness::builder().with_size(egui::vec2(420.0, 520.0)).build_ui_state(
        |ui, (chat, state): &mut (crate::ai_ui::Chat, VaultState)| {
            chat.ui(ui, state);
        },
        (chat, state),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-chat-panel");

    harness.get_by_label("AI");
    harness.get_by_label("Send");
    harness.get_by_label("Insert into the note");
    harness.get_by_label_contains("Resposta da IA");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_sidebar_keeps_the_tree_the_tags_and_the_backlinks_apart() {
    // The three blocks used to be drawn in the same rectangle: nested panels reserve space for
    // themselves and not for what comes after, so the tree and the backlinks overlapped.
    let (scratch, mut state) = demo_vault("side");
    state.open_note(&scratch.0.join("Sepse.md"));
    state.tags_open = true;
    state.rebuild_filtered_tree();
    let mut harness = Harness::builder().with_size(egui::vec2(340.0, 620.0)).build_ui_state(
        |ui, state: &mut VaultState| {
            let _ = crate::sidebar::show(ui, state);
        },
        state,
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-sidebar");

    let note = harness.get_by_label("Protocolo").rect();
    let tags = harness.get_by_label_contains("Tags (").rect();
    let backlinks = harness.get_by_label_contains("Backlinks").rect();
    assert!(note.bottom() <= tags.top() + 1.0, "the tags are drawn over the tree: {note:?} vs {tags:?}");
    assert!(tags.bottom() <= backlinks.top() + 1.0, "the backlinks are drawn over the tags: {tags:?} vs {backlinks:?}");
    assert!(backlinks.bottom() <= 620.0, "the backlinks end outside the sidebar: {backlinks:?}");
}

#[test]
fn a_table_spreads_its_columns_across_the_page() {
    let scratch = Scratch::new(
        "table",
        &[("Tabela.md", "# Tabela\n\n| Quero | Vá para |\n|:--|:--|\n| Aprender do zero, na ordem certa | Trilha de Aprendizado |\n")],
    );
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Tabela.md"));
    state.view = crate::vault_state::ViewMode::Read;
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 520.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-table");

    let first = harness.get_by_label("Quero").rect();
    let second = harness.get_by_label("Vá para").rect();
    assert!((first.top() - second.top()).abs() < 2.0, "the header cells are not on one row: {first:?} vs {second:?}");
    assert!(second.left() - first.left() > 300.0, "the columns did not share the width: {first:?} vs {second:?}");

    // The long label stays on one line and inside a column of its own.
    let cell = harness.get_by_label("Aprender do zero, na ordem certa").rect();
    assert!(cell.height() < 40.0, "the cell wrapped into a tower: {cell:?}");
    assert!(cell.width() > 180.0, "the cell is a sliver: {cell:?}");
}

#[test]
fn the_reading_view_draws_a_tag_as_a_tag() {
    let scratch = Scratch::new("tagchip", &[("Nota.md", "# Nota\n\n#tipo-indice e mais texto\n")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Nota.md"));
    state.view = crate::vault_state::ViewMode::Read;
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(560.0, 320.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            if let crate::editor::EditorAction::FilterTag(tag) = crate::editor::show(ui, state, images) {
                state.tag_filter = Some(tag);
            }
        },
        (state, images),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-tagchip");

    // Clicking the chip filters the sidebar by that tag.
    harness.get_by_label("#tipo-indice").click();
    harness.run_steps(2);
    assert_eq!(harness.state().0.tag_filter.as_deref(), Some("tipo-indice"), "the chip filters the sidebar");
}

#[test]
fn the_reading_view_ticks_a_task_in_the_note() {
    let scratch = Scratch::new("task", &[("Hoje.md", "# Hoje\n\n- [ ] aspirar\n- [x] regar\n"), ("Outra.md", "# Outra\n")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Hoje.md"));
    state.view = crate::vault_state::ViewMode::Read;
    let images = crate::images::Images::default();

    let mut harness = Harness::builder().with_size(egui::vec2(560.0, 420.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            if let crate::editor::EditorAction::ToggleTask(index) = crate::editor::show(ui, state, images) {
                state.toggle_task(index);
            }
        },
        (state, images),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-tasks");

    let boxes: Vec<_> = harness.get_all_by_role(egui::accesskit::Role::CheckBox).collect();
    assert_eq!(boxes.len(), 2, "one box per task");
    boxes[0].click();
    harness.run_steps(2);

    let buffer = &harness.state().0.buffer;
    assert!(buffer.contains("- [x] aspirar"), "{buffer}");
    assert!(buffer.contains("- [x] regar"), "the other task is untouched: {buffer}");
    assert!(harness.state().0.dirty, "the click marks the note edited");
}

#[test]
fn the_help_window_lists_the_keys_and_the_documented_syntax() {
    let mut harness = Harness::builder().with_size(egui::vec2(640.0, 760.0)).build_ui_state(|ui, _: &mut ()| crate::app::help_body(ui), ());
    harness.run_steps(3);
    shot(&mut harness, "ui-help");

    harness.get_by_label("Getting around");
    harness.get_by_label_contains("Save the note");
    harness.get_by_label_contains("Tick or untick a task");
    harness.get_by_label("What a note holds");
    harness.get_by_label_contains("one cloze card per highlight");
    harness.get_by_label("Where things live");
}

#[test]
fn the_connection_map_grows_where_the_reader_clicks_and_opens_what_they_pick() {
    // The map starts at the open note with its direct connections; the badge opens a note's own
    // connections, the note itself opens the note.
    let scratch = Scratch::new(
        "connections",
        &[
            ("Moc.md", "# Moc\n\n[[Ana]] [[Bia]]\n"),
            ("Ana.md", "# Ana\n\n[[Bia]] [[Caio]]\n"),
            ("Bia.md", "# Bia\n\nnada\n"),
            ("Caio.md", "# Caio\n\nnada\n"),
        ],
    );
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Moc.md"));
    let map = crate::connections::Map::default();
    let opened = Rc::new(RefCell::new(None::<PathBuf>));
    let sink = opened.clone();
    let mut harness = Harness::builder().with_size(egui::vec2(760.0, 600.0)).build_ui_state(
        move |ui, (state, map): &mut (VaultState, crate::connections::Map)| match crate::connections::show(ui, state, map) {
            crate::connections::Action::Open(path) => *sink.borrow_mut() = Some(path),
            crate::connections::Action::Start(path) => map.start_from(&path),
            crate::connections::Action::None => {}
        },
        (state, map),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-connections");

    // Moc, Ana and Bia: Caio is two hops away and waits.
    harness.get_by_label("Ana");
    harness.get_by_label("Bia");
    assert!(harness.query_by_label("Caio").is_none(), "a note two hops away is not drawn yet");

    // The badge on Ana opens her connections: Caio joins the map.
    let caio = harness.state().0.files.iter().any(|note| note.name == "Caio");
    assert!(caio);
    harness.get_by_label_contains("Open the connections of Ana").click();
    harness.run_steps(2);
    harness.get_by_label("Caio");

    // And the same badge closes them again.
    harness.get_by_label_contains("Close the connections of Ana").click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Caio").is_none(), "collapsing takes the branch away");

    // Clicking a note opens it (the app is the one that opens, so the test reads the action).
    harness.get_by_label("Bia").click();
    harness.run_steps(2);
    assert_eq!(*opened.borrow(), Some(scratch.0.join("Bia.md")));

    // The same map in a small window: the rings shrink, the labels stay inside the canvas.
    let mut small = crate::connections::Map::default();
    small.start_from(&scratch.0.join("Moc.md"));
    small.toggle(&scratch.0.join("Ana.md"));
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Moc.md"));
    let mut harness = Harness::builder().with_size(egui::vec2(480.0, 420.0)).build_ui_state(
        |ui, (state, map): &mut (VaultState, crate::connections::Map)| {
            crate::connections::show(ui, state, map);
        },
        (state, small),
    );
    harness.run_steps(3);
    shot(&mut harness, "ui-connections-small");
    harness.get_by_label("Caio");
}

#[test]
fn a_busy_note_draws_a_readable_ring() {
    // The case the old vault-wide graph could not do: a note with dozens of connections, drawn at
    // the size it will be read at. Deterministic, so the two sizes can be compared.
    let mut notes: Vec<(String, String)> = Vec::new();
    let links: String = (1..=16).map(|index| format!("[[Filho {index:02}]] ")).collect();
    notes.push(("MOC.md".to_string(), links));
    for index in 1..=16 {
        notes.push((format!("Filho {index:02}.md"), "nada".to_string()));
    }
    for index in 1..=4 {
        notes.push((format!("Fonte {index}.md"), "[[MOC]]".to_string()));
    }
    let borrowed: Vec<(&str, &str)> = notes.iter().map(|(name, text)| (name.as_str(), text.as_str())).collect();
    let scratch = Scratch::new("connections-busy", &borrowed);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("MOC.md"));
    let mut map = crate::connections::Map::default();
    map.start_from(&scratch.0.join("MOC.md"));

    for (width, height) in [(760.0f32, 600.0f32), (520.0, 420.0)] {
        let mut state = VaultState::default();
        state.open(scratch.0.clone()).expect("open the vault");
        state.open_note(&scratch.0.join("MOC.md"));
        let map = map.clone();

        // Every connection is on the map, each with its own place.
        let nodes = crate::connections::layout(&state, &map);
        assert_eq!(nodes.len(), 21, "the note, its 16 links and its 4 backlinks");
        let mut seen: Vec<(i32, i32)> = Vec::new();
        for node in &nodes {
            let cell = ((node.at.0 * width) as i32, (node.at.1 * height) as i32);
            assert!(!seen.contains(&cell), "two notes share a place: {cell:?}");
            seen.push(cell);
        }

        let mut harness = Harness::builder().with_size(egui::vec2(width, height)).build_ui_state(
            |ui, (state, map): &mut (VaultState, crate::connections::Map)| {
                crate::connections::show(ui, state, map);
            },
            (state, map),
        );
        harness.run_steps(3);
        shot(&mut harness, &format!("ui-connections-busy-{}x{}", width as u32, height as u32));
    }
}

#[test]
fn the_connection_map_says_so_when_there_is_no_note_and_when_a_note_has_no_links() {
    let empty = crate::connections::Map::default();
    let mut harness = Harness::builder().with_size(egui::vec2(420.0, 320.0)).build_ui_state(
        |ui, map: &mut crate::connections::Map| {
            let state = VaultState::default();
            crate::connections::show(ui, &state, map);
        },
        empty,
    );
    harness.run_steps(2);
    harness.get_by_label("Open a note to see its connections");

    let scratch = Scratch::new("connections-empty", &[("So.md", "# So\n\nsem links\n")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("So.md"));
    let map = crate::connections::Map::default();
    let mut harness = Harness::builder().with_size(egui::vec2(520.0, 360.0)).build_ui_state(
        |ui, (state, map): &mut (VaultState, crate::connections::Map)| {
            crate::connections::show(ui, state, map);
        },
        (state, map),
    );
    harness.run_steps(2);
    harness.get_by_label_contains("This note has no links yet");
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

#[test]
fn enter_in_the_editor_starts_a_new_line() {
    // The completion popup consumes Enter only while it has something to accept; with no link being
    // typed, Enter belongs to the note.
    let scratch = Scratch::new("enter", &[("Nota.md", "# Nota\n\ncorpo\n")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Nota.md"));
    state.view = crate::vault_state::ViewMode::Edit;
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(700.0, 460.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(3);
    harness.get_by_role(egui::accesskit::Role::MultilineTextInput).click();
    harness.run_steps(2);
    let before = harness.state().0.buffer.matches('\n').count();
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    let after = harness.state().0.buffer.matches('\n').count();
    assert_eq!(after, before + 1, "Enter inserts one line break: {:?}", harness.state().0.buffer);
}

#[test]
fn a_slash_command_at_the_start_of_a_line_opens_a_popup_that_inserts_a_block() {
    let scratch = Scratch::new("slash", &[("Nota.md", "/tab")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Nota.md"));
    state.view = crate::vault_state::ViewMode::Edit;
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(700.0, 460.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(3);
    harness.get_by_role(egui::accesskit::Role::MultilineTextInput).click();
    harness.run_steps(2);
    // The caret goes to the end of the line, right after the command being typed.
    harness.key_press(egui::Key::End);
    harness.run_steps(2);
    shot(&mut harness, "ui-slash");

    // The popup explains the command it offers.
    harness.get_by_label_contains("/table");
    harness.get_by_label_contains("Table");

    // Clicking it replaces the typed command with the block, and selects the first header cell.
    harness.get_by_label_contains("/table").click();
    harness.run_steps(2);
    let buffer = &harness.state().0.buffer;
    assert!(buffer.starts_with("| Col 1 | Col 2 | Col 3 |\n"), "{buffer:?}");
    assert!(buffer.ends_with("| Cell | Cell | Cell |\n"), "{buffer:?}");
}

#[test]
fn enter_takes_the_highlighted_slash_command_instead_of_breaking_the_line() {
    let scratch = Scratch::new("slash-enter", &[("Nota.md", "# Nota\n\n/toc")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Nota.md"));
    state.view = crate::vault_state::ViewMode::Edit;
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(700.0, 460.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(3);
    harness.get_by_role(egui::accesskit::Role::MultilineTextInput).click();
    harness.run_steps(2);
    harness.key_press(egui::Key::End);
    harness.run_steps(2);
    harness.get_by_label_contains("/toc");
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);

    // Enter took the command: the contents replaces `/toc` and no line break was left behind.
    let buffer = &harness.state().0.buffer;
    assert_eq!(buffer, "# Nota\n\n- [[#Nota]]\n", "{buffer:?}");
}

#[test]
fn a_bare_slash_lists_the_whole_catalogue() {
    // The list is the app's insert menu: what a block, a snippet or a date looks like, in the order
    // the popup shows it.
    let scratch = Scratch::new("slash-all", &[("Nota.md", "# Nota\n\n/")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Nota.md"));
    state.view = crate::vault_state::ViewMode::Edit;
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(760.0, 620.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(3);
    harness.get_by_role(egui::accesskit::Role::MultilineTextInput).click();
    harness.run_steps(2);
    harness.key_press(egui::Key::End);
    harness.run_steps(2);
    shot(&mut harness, "ui-slash-all");
    harness.ctx.set_visuals(egui::Visuals::light());
    harness.run_steps(2);
    shot(&mut harness, "ui-slash-all-light");

    for name in ["/h1", "/bold", "/table", "/card"] {
        harness.get_by_label_contains(name);
    }
}

#[test]
fn escape_dismisses_the_slash_popup_and_it_stays_closed_at_that_caret() {
    let scratch = Scratch::new("slash-esc", &[("Nota.md", "/tab")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Nota.md"));
    state.view = crate::vault_state::ViewMode::Edit;
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(700.0, 460.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(3);
    harness.get_by_role(egui::accesskit::Role::MultilineTextInput).click();
    harness.run_steps(2);
    harness.key_press(egui::Key::End);
    harness.run_steps(2);
    harness.get_by_label_contains("/table");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    assert!(harness.query_by_label_contains("/table").is_none(), "Escape closes it, and it does not reopen on the same caret");
    assert_eq!(harness.state().0.buffer, "/tab", "the note is untouched");
}

/// One notch of the wheel, as the app receives it: a small point delta that egui applies at once (a
/// bigger one is smoothed over several frames, which would make a test about "it stayed" measure the
/// animation instead of the fight).
fn wheel_notch<State>(harness: &mut Harness<'_, State>, delta: f32) {
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, delta),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    });
}

/// **`wheel_keep_position`** — the house regression test for scroll surfaces: the wheel must move
/// the area under the pointer and the area must **stay** where the reader left it.
///
/// It exists because of a real bug (`/` in v1.6.0): a `scroll_to_*` asked for on every frame wins
/// against the wheel, and the list climbs back by itself. Every panel with a `ScrollArea` that also
/// scrolls programmatically should call this with a row that stays visible before and after, and a
/// pointer position inside the area. See `docs/planos/licoes-de-porta.md` §UI/scroll.
///
/// `probe` is a label inside the area; `delta` is one notch (negative scrolls the content up, i.e.
/// the reader goes down).
fn wheel_keep_position<State>(harness: &mut Harness<'_, State>, hover: egui::Pos2, probe: &str, delta: f32)
where
    State: 'static,
{
    harness.event(egui::Event::PointerMoved(hover));
    harness.run_steps(1);
    let before = harness.get_by_label_contains(probe).rect().top();
    for _ in 0..4 {
        wheel_notch(harness, delta);
        harness.run_steps(1);
    }
    harness.run_steps(2);
    let moved = harness.get_by_label_contains(probe).rect().top();
    assert!((moved - before).abs() > 8.0, "the wheel did not move the area ({before} -> {moved}); the probe must be a visible row");

    // The regression: the area must not climb back on its own after the wheel stopped.
    harness.run_steps(10);
    let later = harness.get_by_label_contains(probe).rect().top();
    assert!((later - moved).abs() < 1.0, "the area fought the wheel: {moved} -> {later}");
}

#[test]
fn the_arrow_keys_move_the_slash_highlight() {
    // `Down` twice must highlight the third command and `Enter` must take *that* one: a popup that
    // rebuilds its list every frame used to forget how far down the reader was.
    let scratch = Scratch::new("slash-arrows", &[("Nota.md", "/")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Nota.md"));
    state.view = crate::vault_state::ViewMode::Edit;
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(760.0, 620.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(3);
    harness.get_by_role(egui::accesskit::Role::MultilineTextInput).click();
    harness.run_steps(2);
    harness.key_press(egui::Key::End);
    harness.run_steps(2);
    harness.key_press(egui::Key::ArrowDown);
    harness.run_steps(1);
    harness.key_press(egui::Key::ArrowDown);
    harness.run_steps(1);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);

    // h1, h2, h3: the third row is the heading 3.
    assert_eq!(harness.state().0.buffer, "### ", "Enter took the third command, not the first");
}

#[test]
fn the_wheel_is_not_fought_by_the_slash_popup() {
    let scratch = Scratch::new("slash-wheel", &[("Nota.md", "/")]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Nota.md"));
    state.view = crate::vault_state::ViewMode::Edit;
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(760.0, 620.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(3);
    harness.get_by_role(egui::accesskit::Role::MultilineTextInput).click();
    harness.run_steps(2);
    harness.key_press(egui::Key::End);
    harness.run_steps(2);

    // The regression: the wheel walks the list and the list stays there.
    let hover_at = harness.get_by_label_contains("/bold").rect().center();
    wheel_keep_position(&mut harness, hover_at, "/bold", -6.0);
}

#[test]
fn the_find_bar_does_not_fight_the_wheel() {
    let mut body = String::from("# Longa\n\n");
    for index in 1..=60 {
        body.push_str(&format!("Paragrafo {index} com texto suficiente para ocupar uma linha inteira.\n\n"));
    }
    body.push_str("Palavra alvo no fim.\n");
    let scratch = Scratch::new("find-wheel", &[("Longa.md", &body)]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Longa.md"));
    state.view = crate::vault_state::ViewMode::Read;
    state.find.open();
    state.find.query = "alvo".to_string();
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(760.0, 520.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(4);
    // The pointer is over the reading pane; the wheel goes up (away from the hit), and the view must
    // stay there: a hit highlighted by the find bar used to pull the reader back every frame.
    wheel_keep_position(&mut harness, egui::pos2(380.0, 300.0), "Paragrafo 60", 6.0);
}

/// Puts a harness in the light theme the real app would use (styles installed, preference pinned).
fn light<State>(harness: &mut Harness<'_, State>) {
    crate::app::install_styles(&harness.ctx);
    harness.ctx.set_theme(egui::ThemePreference::Light);
}

#[test]
fn the_theme_setting_reaches_the_context_and_the_accent() {
    for (theme, expected) in [(craft_icons::ThemeMode::Light, egui::Theme::Light), (craft_icons::ThemeMode::Dark, egui::Theme::Dark)] {
        let mut harness = Harness::builder().with_size(egui::vec2(320.0, 200.0)).build_ui_state(
            move |ui, applied: &mut bool| {
                if !*applied {
                    crate::app::install_styles(ui.ctx());
                    crate::app::apply_theme(ui.ctx(), theme);
                    *applied = true;
                }
            },
            false,
        );
        harness.run_steps(2);
        assert_eq!(harness.ctx.theme(), expected, "{theme:?}");
        let visuals = harness.ctx.style_of(expected).visuals.clone();
        assert_eq!(visuals.dark_mode, expected == egui::Theme::Dark);
        let accent = if visuals.dark_mode { crate::theme::ACCENT_DARK } else { crate::theme::ACCENT_LIGHT };
        assert_eq!(visuals.hyperlink_color, accent, "{theme:?}: the accent installed on that style");
    }
}

/// `top`, with its alpha, painted over the opaque `bottom` (what `gamma_multiply`'d fills look like).
fn over(top: egui::Color32, bottom: egui::Color32) -> egui::Color32 {
    let keep = u32::from(255 - top.a());
    let mix = |t: u8, b: u8| u8::try_from((u32::from(t) * 255 + u32::from(b) * keep + 127) / 255).unwrap_or(u8::MAX);
    egui::Color32::from_rgb(mix(top.r(), bottom.r()), mix(top.g(), bottom.g()), mix(top.b(), bottom.b()))
}

/// The WCAG 2 contrast ratio between two opaque colours (`(L1 + 0.05) / (L2 + 0.05)`, 1:1 to 21:1).
fn contrast(one: egui::Color32, other: egui::Color32) -> f32 {
    let luminance = |color: egui::Color32| {
        let linear = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    };
    let (bright, dark) = {
        let (one, other) = (luminance(one), luminance(other));
        (one.max(other), one.min(other))
    };
    (bright + 0.05) / (dark + 0.05)
}

/// The accent of both themes is **text** in this app (links, tag pills, the source pane's headings) and
/// never a button's fill, so what the standard holds is the text contrast: over the page, over a text
/// box, and over the tint of itself the chips and highlights paint behind it - at least 4.5:1 (WCAG AA).
#[test]
fn the_accent_is_readable_as_text_in_both_themes() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let ctx = egui::Context::default();
        crate::app::install_styles(&ctx);
        let visuals = ctx.style_of(theme).visuals.clone();
        let accent = crate::theme::accent(visuals.dark_mode);
        // The page (links in the reading view, headings and tags in the source pane) and the text
        // box the source pane is drawn on.
        for (where_, ground) in [("the page", visuals.panel_fill), ("the editor", visuals.extreme_bg_color)] {
            let ratio = contrast(accent, ground);
            assert!(ratio >= 4.5, "{theme:?}: the accent over {where_}: {ratio:.2}:1");
            // The chips: `#tag` is the accent painted on 18% of itself, the `==highlight==` on 22% and
            // the code chip on 14%/12%. (The find bar's match is a 30% tint under the theme's *own* text,
            // so the accent is not what has to be read there.)
            for weight in [0.12, 0.14, 0.18, 0.22] {
                let chip = over(accent.gamma_multiply(weight), ground);
                let ratio = contrast(accent, chip);
                assert!(ratio >= 4.5, "{theme:?}: the accent on its own {:.0}% tint over {where_}: {ratio:.2}:1", weight * 100.0);
            }
        }
    }
}

#[test]
fn the_panels_read_in_the_light_theme() {
    // Half the world runs its desktop in light mode, so the light palette is a release gate, not a
    // taste: one shot per surface that carries a colour of its own.
    let scratch = Scratch::new(
        "light",
        &[
            ("Tudo.md", KITCHEN_SINK),
            ("Nota similar 1.md", "# Nota similar 1\n\nVolta para [[Tudo]] e para [[Outra nota]].\n"),
            ("Outra nota.md", "# Outra nota\n\n[[Tudo]]\n"),
        ],
    );
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Tudo.md"));
    state.view = crate::vault_state::ViewMode::Read;
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(1366.0, 900.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(2);
    light(&mut harness);
    harness.run_steps(3);
    shot(&mut harness, "ui-light-reading");

    // The review card (callout, accent, buttons).
    let mut review = crate::review_ui::Review::default();
    review.start(&harness.state().0, crate::review_ui::Deck::All);
    let mut harness = Harness::builder().with_size(egui::vec2(620.0, 640.0)).build_ui_state(
        |ui, (state, review, images): &mut (VaultState, crate::review_ui::Review, crate::images::Images)| review.ui(ui, state, images),
        (harness.into_state().0, review, crate::images::Images::default()),
    );
    harness.run_steps(2);
    light(&mut harness);
    harness.run_steps(3);
    shot(&mut harness, "ui-light-review");

    // The connection map (violet rings over the note).
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Tudo.md"));
    let mut map = crate::connections::Map::default();
    map.start_from(&scratch.0.join("Tudo.md"));
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 640.0)).build_ui_state(
        |ui, (state, map): &mut (VaultState, crate::connections::Map)| {
            crate::connections::show(ui, state, map);
        },
        (state, map),
    );
    harness.run_steps(2);
    light(&mut harness);
    harness.run_steps(3);
    shot(&mut harness, "ui-light-connections");

    // The calendar (today marked, the month grid).
    let state = (crate::calendar::today_iso(), vec![crate::calendar::today_iso()]);
    let mut harness = Harness::builder().with_size(egui::vec2(360.0, 420.0)).build_ui_state(
        |ui, (month, marked): &mut (String, Vec<String>)| {
            let days = marked.clone();
            crate::calendar::show(ui, month, &|iso: &str| days.iter().any(|day| day == iso), "en");
        },
        state,
    );
    harness.run_steps(2);
    light(&mut harness);
    harness.run_steps(3);
    shot(&mut harness, "ui-light-calendar");

    // The chat panel (strip, composer, chip): the suite component in light.
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Tudo.md"));
    let mut chat = crate::ai_ui::Chat::default();
    chat.open = true;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(460.0, 620.0))
        .build_ui_state(|ui, (state, chat): &mut (VaultState, crate::ai_ui::Chat)| chat.ui(ui, state), (state, chat));
    harness.run_steps(2);
    light(&mut harness);
    harness.run_steps(3);
    shot(&mut harness, "ui-light-chat");

    // The editor (source) beside the reading view: the markdown layouter has colours of its own
    // (links, tags, highlights, fences). A note without emoji: the editor draws the raw text, and the
    // test fonts have no pictograms.
    let source = "# Título\n\nParágrafo com **negrito**, *itálico*, `código`, ==destaque==, [[Nota similar 1]] e #qa.\n\n> citação com [[Tudo]]\n\n- item\n- [ ] tarefa\n\n```rust\nfn main() {}\n```\n";
    let scratch = Scratch::new("light-editor", &[("Fonte.md", source)]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Fonte.md"));
    state.view = crate::vault_state::ViewMode::Split;
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(2);
    light(&mut harness);
    harness.run_steps(3);
    shot(&mut harness, "ui-light-editor");

    // The find bar and the lit matches (a background over the text).
    let mut body = String::from("# Longa\n\n");
    for index in 1..=12 {
        body.push_str(&format!("Paragrafo {index} com a palavra alvo no meio da linha.\n\n"));
    }
    let scratch = Scratch::new("light-find", &[("Longa.md", &body)]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Longa.md"));
    state.view = crate::vault_state::ViewMode::Read;
    state.find.open();
    state.find.query = "alvo".to_string();
    let images = crate::images::Images::default();
    let mut harness = Harness::builder().with_size(egui::vec2(760.0, 520.0)).build_ui_state(
        |ui, (state, images): &mut (VaultState, crate::images::Images)| {
            crate::editor::show(ui, state, images);
        },
        (state, images),
    );
    harness.run_steps(2);
    light(&mut harness);
    harness.run_steps(3);
    shot(&mut harness, "ui-light-find");

    // The vault search panel.
    let (scratch2, mut state) = demo_vault("light-search");
    state.open_note(&scratch2.0.join("Sepse.md"));
    state.search.open = true;
    state.search.query = "lactato".to_string();
    state.run_search_now();
    let mut harness = Harness::builder().with_size(egui::vec2(420.0, 560.0)).build_ui_state(
        |ui, state: &mut VaultState| {
            let _ = panels::search(ui, state);
        },
        state,
    );
    harness.run_steps(2);
    light(&mut harness);
    harness.run_steps(3);
    shot(&mut harness, "ui-light-search");

    // The AI connections dialog (a form: fields, hints, buttons).
    let mut chat = crate::ai_ui::Chat::default();
    chat.open = true;
    chat.settings_open = true;
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Tudo.md"));
    let mut harness = Harness::builder().with_size(egui::vec2(760.0, 620.0)).build_ui_state(
        |ui, (state, chat): &mut (VaultState, crate::ai_ui::Chat)| {
            chat.ui(ui, state);
            // The dialog is drawn by the app, outside the panel (as in `app.rs`).
            let _ = chat.settings(ui);
        },
        (state, chat),
    );
    harness.run_steps(2);
    light(&mut harness);
    harness.run_steps(3);
    shot(&mut harness, "ui-light-ai-dialog");
}

#[test]
fn the_open_note_exports_as_a_pdf_with_selectable_text() {
    // The menu writes what the reader sees: the same blocks, on A4 pages, with the fonts embedded.
    let scratch = Scratch::new("pdf", &[("Tudo.md", KITCHEN_SINK)]);
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Tudo.md"));
    state.refresh_preview();
    let path = std::env::temp_dir().join(format!("localobsidian-export-{}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);

    let report = crate::app::export_note_pdf(state.preview(), "Tudo", &state, &path).expect("the export");
    assert!(report.pages >= 2, "the kitchen sink does not fit on one page: {}", report.pages);
    let bytes = std::fs::read(&path).expect("the file");
    assert_eq!(bytes.len(), report.bytes);
    assert!(bytes.starts_with(b"%PDF-"));
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/ToUnicode"), "the glyphs carry a Unicode map (the text is selectable)");
    assert!(text.contains("/FontFile2") || text.contains("/FontFile"), "the faces are embedded");

    // The QA run keeps the file for the reviewer to open and look at.
    if std::env::var_os("LOCALOBSIDIAN_QA_RENDER").is_some() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/qa-f7");
        let _ = std::fs::create_dir_all(&directory);
        let _ = std::fs::copy(&path, directory.join("export-note.pdf"));
    }
    let _ = std::fs::remove_file(&path);
}

/// A 1×1 PNG: the smallest picture a note can embed, enough for the export to have real bytes.
const A_PIXEL: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 1, 115, 82, 71,
    66, 0, 174, 206, 28, 233, 0, 0, 0, 4, 103, 65, 77, 65, 0, 0, 177, 143, 11, 252, 97, 5, 0, 0, 0, 9, 112, 72, 89, 115, 0, 0, 14, 195, 0, 0, 14,
    195, 1, 199, 111, 168, 100, 0, 0, 0, 13, 73, 68, 65, 84, 24, 87, 99, 248, 207, 192, 240, 31, 0, 5, 0, 1, 255, 166, 92, 155, 93, 0, 0, 0, 0, 73,
    69, 78, 68, 174, 66, 96, 130,
];

#[test]
fn the_exported_pdf_carries_the_picture_the_link_and_the_nested_list() {
    // The app's side of the phase-2 export: the picture is found in the vault (`_attachments/`),
    // its bytes reach the document, a web link becomes an annotation and the sub-list is drawn
    // under its item (the parser and the crate tests check the shape; here it is the whole path).
    let note = "# Nota\n\n- pai\n  - filho\n\n[link](https://example.com/nota)\n\n![[figura-qa.png]]\n";
    let scratch = Scratch::new("pdf-pictures", &[("Nota.md", note)]);
    let attachments = scratch.0.join("_attachments");
    std::fs::create_dir_all(&attachments).expect("the attachments folder");
    std::fs::write(attachments.join("figura-qa.png"), A_PIXEL).expect("the picture");
    let mut state = VaultState::default();
    state.open(scratch.0.clone()).expect("open the vault");
    state.open_note(&scratch.0.join("Nota.md"));
    state.refresh_preview();
    let path = std::env::temp_dir().join(format!("localobsidian-export-pictures-{}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);

    crate::app::export_note_pdf(state.preview(), "Nota", &state, &path).expect("the export");
    let bytes = std::fs::read(&path).expect("the file");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Subtype /Image") || text.contains("/Subtype/Image"), "the picture is embedded");
    assert!(text.contains("/Link"), "the link is clickable");
    assert!(text.contains("https://example.com/nota"), "the annotation carries its target");
    let _ = std::fs::remove_file(&path);
}

// ---- The suite's shell: theme, Settings, the command palette --------------------------------------

type Shell = Harness<'static, Option<crate::app::LocalApp>>;

/// The whole window over a headless context: the real top bar, the menus and the windows, with nothing written to
/// the user's settings file.
fn shell(theme: craft_icons::ThemeMode) -> Shell {
    shell_sized(theme, 1000.0, 640.0)
}

/// [`shell`] at another size, for the tests that hold the window to its minimum.
fn shell_sized(theme: craft_icons::ThemeMode, width: f32, height: f32) -> Shell {
    let mut harness = Harness::builder().with_size(egui::vec2(width, height)).build_ui_state(
        move |ui, app: &mut Option<crate::app::LocalApp>| {
            let app = app.get_or_insert_with(|| {
                crate::app::install_styles(ui.ctx());
                let prefs = crate::prefs::Prefs { theme, ..Default::default() };
                crate::app::LocalApp::for_test(ui.ctx(), prefs)
            });
            app.run_logic(ui.ctx());
            app.show(ui);
        },
        None,
    );
    harness.run();
    harness
}

/// The theme, whether Settings is open and whether the palette is open.
fn shell_state(harness: &Shell) -> (craft_icons::ThemeMode, bool, bool) {
    harness.state().as_ref().expect("the app is built on the first frame").shell_state()
}

/// View > Theme > `choice`: a real click through the menus.
fn pick_theme(harness: &mut Shell, choice: &str) {
    harness.get_by_label("View").click();
    harness.run();
    harness.get_by_label("Theme").click();
    harness.run();
    harness.get_by_label(choice).click();
    harness.run();
}

#[test]
fn the_view_menu_changes_the_theme_to_each_of_the_suites_three() {
    use craft_icons::ThemeMode;
    let mut harness = shell(ThemeMode::System);
    assert_eq!(shell_state(&harness).0, ThemeMode::System);
    pick_theme(&mut harness, "Dark");
    assert_eq!(shell_state(&harness).0, ThemeMode::Dark);
    assert_eq!(harness.ctx.theme(), egui::Theme::Dark);
    pick_theme(&mut harness, "Light");
    assert_eq!(shell_state(&harness).0, ThemeMode::Light);
    assert_eq!(harness.ctx.theme(), egui::Theme::Light);
    pick_theme(&mut harness, "Automatic");
    assert_eq!(shell_state(&harness).0, ThemeMode::System);
    shot(&mut harness, "casca-theme-menu");
}

#[test]
fn the_themes_button_walks_automatic_light_dark() {
    use craft_icons::ThemeMode;
    let mut harness = shell(ThemeMode::System);
    for expected in [ThemeMode::Light, ThemeMode::Dark, ThemeMode::System] {
        harness.get_by_label_contains("Theme:").click();
        harness.run();
        assert_eq!(shell_state(&harness).0, expected);
    }
}

#[test]
fn the_gear_and_file_settings_open_settings_with_the_language_and_the_theme() {
    use craft_icons::ThemeMode;
    let mut harness = shell(ThemeMode::Dark);
    assert!(!shell_state(&harness).1);
    harness.get_by_label("Settings").click();
    harness.run();
    assert!(shell_state(&harness).1, "the gear opens Settings");
    assert!(harness.query_all_by_label("Language").next().is_some(), "the language lives in Settings");
    assert!(harness.query_all_by_label("Theme").next().is_some(), "and so does the theme");
    shot(&mut harness, "casca-settings");

    // The theme row of Settings switches the mode too.
    harness.get_by_label("Light").click();
    harness.run();
    assert_eq!(shell_state(&harness).0, ThemeMode::Light);

    // Closed from its own X, then opened again from the File menu.
    let mut harness = shell(ThemeMode::Dark);
    harness.get_by_label("File").click();
    harness.run();
    harness.get_by_label("Settings…").click();
    harness.run();
    assert!(shell_state(&harness).1, "File > Settings… opens it too");
}

#[test]
fn the_search_icon_and_ctrl_k_open_the_palette_and_run_a_command() {
    use craft_icons::ThemeMode;
    let mut harness = shell(ThemeMode::Dark);
    assert!(!shell_state(&harness).2);
    harness.get_by_label_contains("Search commands").click();
    harness.run();
    assert!(shell_state(&harness).2, "the magnifier opens the palette");
    shot(&mut harness, "casca-palette");
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(!shell_state(&harness).2, "Escape closes it");

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    harness.run();
    assert!(shell_state(&harness).2, "Ctrl+K opens it");

    // Typing narrows the list and Enter runs the first match: the same Settings the gear opens.
    harness.get_by_role(egui::accesskit::Role::TextInput).type_text("Settings");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    let (_, settings, palette) = shell_state(&harness);
    assert!(settings, "the command ran through the menu's code");
    assert!(!palette, "and the palette closed");
}

#[test]
fn ctrl_shift_k_is_the_flashcards_shortcut_and_not_the_palette() {
    use craft_icons::ThemeMode;
    let mut harness = shell(ThemeMode::Dark);
    harness.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::K);
    harness.run();
    assert!(!shell_state(&harness).2, "egui matches Ctrl+K with Shift held; the flashcards key takes it first");
}

#[test]
fn the_palette_lists_every_menu_and_greys_what_needs_a_vault() {
    use craft_icons::ThemeMode;
    let mut harness = shell(ThemeMode::Dark);
    let commands = {
        let app = harness.state().as_ref().expect("built");
        app.palette_commands(&harness.ctx)
    };
    let find = |label: &str| commands.iter().find(|command| command.label == label).unwrap_or_else(|| panic!("{label} is not in the palette"));
    for menu in ["File", "View", "Review", "AI", "Help"] {
        assert!(commands.iter().any(|command| command.group == menu), "{menu} has commands in the palette");
    }
    assert!(find("Settings…").enabled);
    assert!(find("Exit").enabled);
    assert!(find("Check for Updates…").enabled);
    assert!(!find("New note…").enabled, "no vault yet: listed but greyed, like the menu");
    assert!(!find("Create flashcards from this note").enabled);
    assert_eq!(find("Save").shortcut, harness.ctx.format_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S)));
    for theme in ["Theme: Automatic", "Theme: Light", "Theme: Dark"] {
        assert!(find(theme).enabled);
    }
    harness.run();
}

/// Visual QA (ignored): the top bar, Settings and the palette in both themes, over a vault with notes. The PNGs land in
/// `target/qa-casca/`; the render needs a GPU.
#[test]
#[ignore = "visual QA: renders the window with the suite's shell in both themes"]
fn casca_shots() {
    use craft_icons::ThemeMode;
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/qa-casca");
    std::fs::create_dir_all(&directory).expect("the QA folder");
    let (scratch, _state) = demo_vault("casca-shots");
    for (name, mode) in [("light", ThemeMode::Light), ("dark", ThemeMode::Dark)] {
        let take = |harness: &mut Shell, what: &str| {
            harness.run();
            let image = harness.render().expect("the render");
            image.save(directory.join(format!("{name}-{what}.png"))).expect("the PNG");
        };
        let open = |harness: &mut Shell| {
            if let Some(app) = harness.state_mut().as_mut() {
                app.open_vault_for_test(scratch.0.clone());
            }
            harness.run();
        };

        let mut harness = shell(mode);
        open(&mut harness);
        take(&mut harness, "1-barra");

        harness.get_by_label("File").click();
        harness.run();
        take(&mut harness, "2-menu-arquivo");
        harness.key_press(egui::Key::Escape);

        let mut harness = shell(mode);
        open(&mut harness);
        harness.get_by_label("Settings").click();
        take(&mut harness, "3-configuracoes");

        let mut harness = shell(mode);
        open(&mut harness);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
        harness.run();
        // The box has the focus on opening: the typing goes to it.
        harness.event(egui::Event::Text("re".to_string()));
        take(&mut harness, "4-paleta");
    }
}

/// Every node of the accessibility tree stays inside a window `width` wide (a node a screen reader can
/// point at that sits past the edge is a control the user cannot see). The width is what the standard
/// holds at the window's minimum; height is the scroll's business - a window taller than the screen
/// scrolls its content (the help window at 440 pt), and a node past the fold is reached by scrolling.
fn assert_inside_the_window(harness: &Shell, what: &str, width: f32) {
    for node in harness.root().children_recursive() {
        let role = node.accesskit_node().role();
        let Some(rect) = node.accesskit_node().bounding_box() else { continue };
        // A panel's resize handle is a 6 pt strip centred on the panel's border: half of it is meant to
        // sit on the window's edge (egui draws it; nothing of ours can move it inward).
        if rect.width() <= 0.0 || role == egui::accesskit::Role::Splitter {
            continue;
        }
        assert!(rect.x0 >= -1.0 && rect.x1 <= f64::from(width) + 1.0, "{what}: {role:?} leaves the {width:.0} pt window: {rect:?}");
    }
}

/// A vault with notes (the same fixture the shell tests read), open in `harness`.
fn open_the_vault(harness: &mut Shell, vault: &Scratch) {
    if let Some(app) = harness.state_mut().as_mut() {
        app.open_vault_for_test(vault.0.clone());
    }
    harness.run();
}

/// The window's minimum size, from `main.rs`'s `with_min_inner_size`: the width and height every
/// window and dialog has to live in.
const NARROW: (f32, f32) = (720.0, 440.0);

/// Each window and dialog of the app, opened over a vault with notes, in the window's **minimum
/// size**: no node of the accessibility tree may leave the window. This is the test the "two
/// widths" rule of the craft standard asks for; the palette and the update card are part of it.
#[test]
fn every_window_fits_the_narrowest_window() {
    use crate::app::WindowForTest;
    use craft_icons::ThemeMode;
    let windows = [
        ("new note", WindowForTest::NewNote, "Create"),
        ("rename", WindowForTest::Rename, "Rename note"),
        ("diagnostics", WindowForTest::Diagnostics, "Vault diagnostics"),
        ("connections", WindowForTest::Connections, "Connections"),
        ("calendar", WindowForTest::Calendar, "Calendar"),
        ("help", WindowForTest::Help, "Help"),
        ("about", WindowForTest::About, "About Local Obsidian"),
        ("settings", WindowForTest::Settings, "Settings"),
        ("ai connection", WindowForTest::AiConnection, "AI connection"),
        ("flashcards", WindowForTest::Flashcards, "Cards per section"),
        ("palette", WindowForTest::Palette, ""),
        ("update", WindowForTest::Update, "DOWNLOAD"),
    ];
    for (name, window, anchor) in windows {
        for theme in [ThemeMode::Dark, ThemeMode::Light] {
            let (vault, _state) = demo_vault(&format!("narrow-{}-{}", name.replace(' ', "-"), theme.id()));
            let mut harness = shell_sized(theme, NARROW.0, NARROW.1);
            open_the_vault(&mut harness, &vault);
            if let Some(app) = harness.state_mut().as_mut() {
                app.open_window_for_test(window);
            }
            harness.run_steps(3);
            shot(&mut harness, &format!("narrow-{}-{}", name.replace(' ', "-"), theme.id()));
            // The window is really on screen before its geometry means anything.
            if name == "palette" {
                assert!(harness.query_all_by_role(egui::accesskit::Role::ListBoxOption).next().is_some(), "the palette lists the commands");
            } else {
                assert!(harness.query_all_by_label(anchor).next().is_some(), "{name}: nothing on screen says {anchor:?}");
            }
            assert_inside_the_window(&harness, name, NARROW.0);
        }
    }
}

/// The top bar itself, over a vault with a note open, in the narrowest window: the menus and the
/// suite's three buttons (gear, theme, search) share the strip without either leaving it.
#[test]
fn the_top_bar_fits_the_narrowest_window() {
    use craft_icons::ThemeMode;
    for theme in [ThemeMode::Dark, ThemeMode::Light] {
        let (vault, _state) = demo_vault(&format!("narrow-bar-{}", theme.id()));
        let mut harness = shell_sized(theme, NARROW.0, NARROW.1);
        open_the_vault(&mut harness, &vault);
        harness.run_steps(2);
        shot(&mut harness, &format!("narrow-bar-{}", theme.id()));
        assert_inside_the_window(&harness, "top bar", NARROW.0);
        for label in ["File", "View", "Review", "AI", "Help"] {
            let menu = harness.get_by_label(label).rect();
            assert!(menu.right() <= NARROW.0, "the {label} menu leaves the window: {menu:?}");
        }
        let gear = harness.get_by_label("Settings").rect();
        let theme_button = harness.get_by_label_contains("Theme:").rect();
        let search = harness.get_by_label_contains("Search commands").rect();
        assert!(gear.right() <= NARROW.0 && theme_button.right() <= NARROW.0 && search.right() <= NARROW.0, "a suite button leaves the window");
        assert!(search.right() < theme_button.left(), "search and theme do not overlap: {search:?} vs {theme_button:?}");
        assert!(theme_button.right() < gear.left(), "theme and gear do not overlap: {theme_button:?} vs {gear:?}");
    }
}
