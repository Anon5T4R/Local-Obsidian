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
        // other), and the line under a callout is a paragraph of the note, not the callout's body.
        let first_block = harness.get_by_label_contains("tag: #qa").rect();
        let second_block = harness.get_by_label_contains("tag: #tag-depois-do-callout").rect();
        let gap = second_block.top() - first_block.bottom();
        assert!(gap >= 28.0, "{width}x{height}: two blocks touch ({gap:.0} px of gap)");

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
    let note = "# T\n\n```query\ntag: #qa\nsort: titulo\n```\n\n```query\ntag: #outra\nsort: titulo\n```\n";
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

    let first = harness.get_by_label_contains("tag: #qa").rect();
    let second = harness.get_by_label_contains("tag: #outra").rect();
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
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1366.0, 768.0))
        // The source pane is a plain text box with no accessible name; the check is about labels,
        // and this test is about geometry.
        .with_accessibility_check(false)
        .build_ui_state(
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
        let mut harness = Harness::builder()
            .with_size(egui::vec2(width, height))
            // The fields are named by the label above them, not by an accessible name of their own.
            .with_accessibility_check(false)
            .build_ui_state(
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
            let mut harness = Harness::builder().with_size(egui::vec2(width, height)).with_accessibility_check(false).build_ui_state(
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
    let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).with_accessibility_check(false).build_ui_state(
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

    // The scope is the note's own name, and the counts belong to that deck.
    harness.get_by_label("MOC - Medicina");
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
