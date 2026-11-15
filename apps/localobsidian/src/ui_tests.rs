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
