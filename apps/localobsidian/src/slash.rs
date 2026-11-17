//! The `/commands` popup: the block, the snippet or the date a note starts with, inserted at the
//! caret.
//!
//! The catalogue follows the upstream's single insert list (`utils/insertables.ts` in the Electron
//! app): one table of commands, each with its aliases, the Markdown it inserts, and the placeholder
//! the caret selects so typing types over it. A command is recognized at the start of a word - the
//! beginning of a line or right after whitespace, which is what makes `/card` work inside a list of
//! a callout - and only while the name is being typed: a space or any punctuation closes the popup,
//! so a note can still say "e/ou" and a date `12/2026` never opens a menu.
//!
//! The keys and the position come from the same place as the `[[wikilink` autocomplete (see
//! [`crate::completions`] and [`crate::editor_keys`]): drawn at the caret, `Up`/`Down` walk,
//! `Enter`/`Tab` take, `Escape` dismisses, and the keys are consumed so the note gains no newline
//! behind the popup.
//!
//! The insertion is plain Markdown, the format every other Markdown app reads. A block snippet that
//! is taken mid-line gets the newline it needs first.

use localobsidian_editor as editor;
use localobsidian_md::outline;

/// The longest name the popup still reads as a command being typed.
const MAX_TYPED: usize = 24;

/// One command of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// Stable identifier, also what the insertion switches on (`toc`, `date` and `time` are built at
    /// insertion time, not from the snippet).
    pub id: &'static str,
    /// What is typed after `/`. The first one is the name the popup shows.
    pub aliases: &'static [&'static str],
    /// The one line the popup shows beside the name (a translation key).
    pub hint: &'static str,
    /// The Markdown inserted (`""` for the commands built at insertion time).
    pub snippet: &'static str,
    /// The placeholder selected after the insertion, so typing replaces it.
    pub select: Option<&'static str>,
    /// Where the caret goes when nothing is selected, counted from the insertion start.
    pub caret: usize,
    /// Whether the snippet is a block (it needs a line of its own to render).
    pub block: bool,
}

/// The commands, in the order the popup shows them.
pub const COMMANDS: &[Command] = &[
    // Basics
    Command { id: "h1", aliases: &["h1", "title"], hint: "Heading 1", snippet: "# ", select: None, caret: 2, block: true },
    Command { id: "h2", aliases: &["h2"], hint: "Heading 2", snippet: "## ", select: None, caret: 3, block: true },
    Command { id: "h3", aliases: &["h3"], hint: "Heading 3", snippet: "### ", select: None, caret: 4, block: true },
    Command { id: "bold", aliases: &["bold", "b"], hint: "Bold", snippet: "**text**", select: Some("text"), caret: 0, block: false },
    Command { id: "italic", aliases: &["italic", "i"], hint: "Italic", snippet: "*text*", select: Some("text"), caret: 0, block: false },
    Command { id: "strike", aliases: &["strike"], hint: "Struck through", snippet: "~~text~~", select: Some("text"), caret: 0, block: false },
    Command {
        id: "highlight",
        aliases: &["highlight", "mark"],
        hint: "Highlight",
        snippet: "==text==",
        select: Some("text"),
        caret: 0,
        block: false,
    },
    Command { id: "inline", aliases: &["inline"], hint: "Inline code", snippet: "`code`", select: Some("code"), caret: 0, block: false },
    Command { id: "comment", aliases: &["comment"], hint: "Comment", snippet: "%%text%%", select: Some("text"), caret: 0, block: false },
    // Structure
    Command { id: "list", aliases: &["list", "ul"], hint: "Bullet list", snippet: "- ", select: None, caret: 2, block: true },
    Command { id: "numlist", aliases: &["numlist", "ol"], hint: "Numbered list", snippet: "1. ", select: None, caret: 3, block: true },
    Command { id: "task", aliases: &["task", "todo", "check"], hint: "Task", snippet: "- [ ] ", select: None, caret: 6, block: true },
    Command {
        id: "table",
        aliases: &["table"],
        hint: "Table",
        snippet: "| Col 1 | Col 2 | Col 3 |\n| --- | --- | --- |\n| Cell | Cell | Cell |\n",
        select: Some("Col 1"),
        caret: 0,
        block: true,
    },
    Command { id: "code", aliases: &["code"], hint: "Code block", snippet: "```\n\n```\n", select: None, caret: 4, block: true },
    Command { id: "quote", aliases: &["quote"], hint: "Quote", snippet: "> ", select: None, caret: 2, block: true },
    Command { id: "hr", aliases: &["hr"], hint: "Horizontal rule", snippet: "---\n", select: None, caret: 4, block: true },
    // Links and media
    Command {
        id: "wikilink",
        aliases: &["wikilink", "note", "nota"],
        hint: "Link to another note",
        snippet: "[[]]",
        select: None,
        caret: 2,
        block: false,
    },
    Command { id: "embed", aliases: &["embed"], hint: "Embed another note", snippet: "![[Note]]", select: Some("Note"), caret: 0, block: true },
    Command { id: "link", aliases: &["link", "url"], hint: "Web link", snippet: "[text](https://)", select: Some("text"), caret: 0, block: false },
    Command { id: "image", aliases: &["image", "img"], hint: "Image", snippet: "![alt](url)", select: Some("alt"), caret: 0, block: false },
    // Study
    Command {
        id: "card",
        aliases: &["card", "flashcard"],
        hint: "Flashcard",
        snippet: "> [!card]- Question\n> Answer\n",
        select: Some("Question"),
        caret: 0,
        block: true,
    },
    Command {
        id: "cloze",
        aliases: &["cloze", "gap"],
        hint: "Cloze flashcard",
        snippet: "> [!card] Title\n> A sentence with ==the hidden term==.\n",
        select: Some("the hidden term"),
        caret: 0,
        block: true,
    },
    Command { id: "toc", aliases: &["toc", "contents"], hint: "Table of contents", snippet: "", select: None, caret: 0, block: true },
    // Callouts
    Command {
        id: "callout",
        aliases: &["callout"],
        hint: "Callout",
        snippet: "> [!note] Title\n> Text\n",
        select: Some("Title"),
        caret: 0,
        block: true,
    },
    Command { id: "tip", aliases: &["tip"], hint: "Tip", snippet: "> [!tip] Title\n> Text\n", select: Some("Title"), caret: 0, block: true },
    Command {
        id: "warning",
        aliases: &["warning"],
        hint: "Warning",
        snippet: "> [!warning] Title\n> Text\n",
        select: Some("Title"),
        caret: 0,
        block: true,
    },
    Command {
        id: "danger",
        aliases: &["danger"],
        hint: "Danger",
        snippet: "> [!danger] Title\n> Text\n",
        select: Some("Title"),
        caret: 0,
        block: true,
    },
    // Data
    Command {
        id: "frontmatter",
        aliases: &["frontmatter", "yaml"],
        hint: "Frontmatter",
        snippet: "---\ntipo: \naliases:\n  - \ntags:\n  - \n---\n",
        select: None,
        caret: 10,
        block: true,
    },
    Command { id: "date", aliases: &["date", "today"], hint: "Today's date", snippet: "", select: None, caret: 0, block: false },
    Command { id: "time", aliases: &["time", "now"], hint: "Time", snippet: "", select: None, caret: 0, block: false },
];

/// A command being typed, and the commands that could complete it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slash {
    /// Where the `/typed` starts and ends in the note.
    pub range: editor::Selection,
    /// What was typed after `/`.
    pub typed: String,
    /// The commands that answer to what was typed, best first.
    pub matches: Vec<&'static Command>,
    /// The command `Up`/`Down` is on.
    pub index: usize,
}

impl Slash {
    /// The command that `Enter` would take.
    pub fn selected(&self) -> Option<&'static Command> {
        self.matches.get(self.index).copied()
    }
}

/// The byte offset where the word being typed starts: right after the last whitespace before the
/// caret.
fn word_start(line: &str) -> usize {
    let mut start = 0;
    for (index, character) in line.char_indices() {
        if character.is_whitespace() {
            start = index + character.len_utf8();
        }
    }
    start
}

/// Whether a command could be being typed at `caret`: the word under it starts with `/`.
///
/// The editor asks this first, so the outline is not rebuilt on every keystroke of a note that is
/// not writing a command.
pub fn maybe(buffer: &str, caret: usize) -> bool {
    let start = editor::line_start(buffer, caret);
    let line = &buffer[start..caret];
    line[word_start(line)..].starts_with('/')
}

/// The command being typed at `caret`, if the caret is where a command can start.
///
/// `has_headings` is what makes `/toc` offrable: a note with no headings has no contents to list.
pub fn for_caret(buffer: &str, caret: usize, has_headings: bool) -> Option<Slash> {
    let start = editor::line_start(buffer, caret);
    let line = &buffer[start..caret];
    let from = start + word_start(line);
    let typed = line[word_start(line)..].strip_prefix('/')?;
    if typed.chars().count() > MAX_TYPED || !typed.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    if in_fence(buffer, start) {
        return None;
    }
    let needle = typed.to_ascii_lowercase();
    let matches = search(&needle, has_headings);
    if matches.is_empty() {
        return None;
    }
    Some(Slash { range: editor::Selection::new(from, caret), typed: typed.to_string(), matches, index: 0 })
}

/// The commands `typed` could be, best first: a name that starts with it (any alias), then one whose
/// identifier contains it - so `/callout` finds the callout family and typing a name never ranks
/// below a substring hit.
fn search(typed: &str, has_headings: bool) -> Vec<&'static Command> {
    let offered = |command: &'static Command| command.id != "toc" || has_headings;
    let starts: Vec<&'static Command> =
        COMMANDS.iter().filter(|command| offered(command) && command.aliases.iter().any(|alias| alias.starts_with(typed))).collect();
    let rest: Vec<&'static Command> =
        COMMANDS.iter().filter(|command| !starts.contains(command) && offered(command) && command.id.contains(typed)).collect();
    starts.into_iter().chain(rest).collect()
}

/// Whether `at` (the start of a line) sits inside a fenced code block.
///
/// A fence is a line whose first non-space characters are three backticks or three tildes: inside
/// one, a `/` is code being written, not a command.
fn in_fence(buffer: &str, at: usize) -> bool {
    let fences = buffer[..at].lines().filter(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("```") || trimmed.starts_with("~~~")
    });
    fences.count() % 2 == 1
}

/// Moves the highlight by `delta`, wrapping around the list.
pub fn move_by(slash: &mut Slash, delta: isize) {
    let len = slash.matches.len();
    if len == 0 {
        return;
    }
    slash.index = (slash.index as isize + delta).rem_euclid(len as isize) as usize;
}

/// The text `command` inserts, and the selection it leaves (a caret, or the placeholder to type
/// over). `None` when there is nothing to insert: `/toc` in a note without headings.
///
/// `headings` feeds `/toc`; `today` and `now` feed `/date` and `/time`, which are read at insertion
/// time - a note written from a machine left running for a week would otherwise carry the wrong day.
pub fn insertion(command: &Command, headings: &[outline::Heading], today: &str, now: &str) -> Option<(String, editor::Selection)> {
    match command.id {
        "toc" => {
            let mut text = String::new();
            for heading in headings {
                let level = usize::from(heading.level).max(1);
                for _ in 1..level {
                    text.push_str("  ");
                }
                text.push_str("- [[#");
                text.push_str(heading.text.trim());
                text.push_str("]]\n");
            }
            let end = text.len();
            if end == 0 {
                return None;
            }
            Some((text, editor::Selection::caret(end)))
        }
        "date" => Some((today.to_string(), editor::Selection::caret(today.len()))),
        "time" => Some((now.to_string(), editor::Selection::caret(now.len()))),
        _ => {
            let text = command.snippet.to_string();
            let selection = match command.select {
                Some(placeholder) => {
                    let start = text.find(placeholder)?;
                    editor::Selection::new(start, start + placeholder.len())
                }
                None => editor::Selection::caret(command.caret.min(text.len())),
            };
            Some((text, selection))
        }
    }
}

/// The note text after taking `command`, and where the caret (or the selection) goes.
///
/// A block snippet taken mid-line gets a newline first: a table or a callout inserted in the middle
/// of a sentence would not render.
pub fn accept(
    buffer: &str,
    slash: &Slash,
    command: &Command,
    headings: &[outline::Heading],
    today: &str,
    now: &str,
) -> Option<(String, editor::Selection)> {
    let (insertion, selection) = insertion(command, headings, today, now)?;
    let range = slash.range.clamped(buffer);
    let line_start = buffer[..range.start].rfind('\n').map_or(0, |at| at + 1);
    let needs_break = command.block && !buffer[line_start..range.start].trim().is_empty();
    let prefix = if needs_break { "\n" } else { "" };

    let mut out = String::with_capacity(buffer.len() + prefix.len() + insertion.len());
    out.push_str(&buffer[..range.start]);
    out.push_str(prefix);
    out.push_str(&insertion);
    out.push_str(&buffer[range.end..]);

    let shift = range.start + prefix.len();
    Some((out, editor::Selection::new(shift + selection.start, shift + selection.end)))
}

/// Draws the popup at `at`. Returns the command the user clicked.
pub fn show(ui: &egui::Ui, slash: &Slash, at: egui::Pos2) -> Option<usize> {
    let mut clicked = None;
    egui::Area::new(ui.scope_id().with("slash-command")).order(egui::Order::Foreground).fixed_pos(at).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_min_width(320.0);
            ui.set_max_width(420.0);
            // The catalogue is long: a bare `/` lists every command, and the popup scrolls instead of
            // running off the window.
            egui::ScrollArea::vertical().id_salt("slash-command-rows").max_height(280.0).auto_shrink([false, false]).show(ui, |ui| {
                for (index, command) in slash.matches.iter().enumerate() {
                    let name = command.aliases.first().copied().unwrap_or(command.id);
                    let hint = crate::i18n::tr(command.hint);
                    let response = ui.selectable_label(index == slash.index, row(ui, name, hint));
                    if index == slash.index {
                        // Walking the list with the keyboard keeps the highlighted row in view.
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.clicked() {
                        clicked = Some(index);
                    }
                }
            });
        });
    });
    clicked
}

/// One row of the popup: the command in the body font, its explanation small and weak beside it.
fn row(ui: &egui::Ui, name: &str, hint: &str) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let name_format =
        egui::TextFormat { font_id: egui::TextStyle::Body.resolve(ui.style()), color: egui::Color32::PLACEHOLDER, ..Default::default() };
    job.append(&format!("/{name}"), 0.0, name_format);
    let hint_format =
        egui::TextFormat { font_id: egui::TextStyle::Small.resolve(ui.style()), color: ui.visuals().weak_text_color(), ..Default::default() };
    job.append(&format!("   {hint}"), 0.0, hint_format);
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headings() -> Vec<outline::Heading> {
        outline::outline("# Sepse\n\n## Conduta\n\n### Antibiótico\n")
    }

    fn slash(text: &str) -> Option<Slash> {
        for_caret(text, text.len(), true)
    }

    fn command(id: &str) -> &'static Command {
        let found = COMMANDS.iter().find(|command| command.id == id);
        assert!(found.is_some(), "no command {id}");
        found.unwrap_or(&COMMANDS[0])
    }

    /// The insertion of `id` at the end of `buffer` (the whole note for the simple cases).
    fn take(buffer: &str, id: &str) -> (String, editor::Selection) {
        let typed = for_caret(buffer, buffer.len(), true).expect("a slash command");
        let (text, selection) = accept(buffer, &typed, command(id), &headings(), "2026-11-17", "09:41").expect("an insertion");
        (text, selection)
    }

    #[test]
    fn a_command_at_the_start_of_a_line_lists_the_commands_that_fit() {
        let typed = slash("/h2").expect("a slash command");
        assert_eq!(typed.typed, "h2");
        assert_eq!(typed.matches.iter().map(|command| command.id).collect::<Vec<_>>(), ["h2"]);
        assert_eq!(typed.selected().map(|command| command.id), Some("h2"));
    }

    #[test]
    fn an_alias_matches_and_the_first_name_is_the_one_shown() {
        let typed = slash("/flash").expect("a slash command");
        assert_eq!(typed.matches.first().map(|command| command.id), Some("card"));
        assert_eq!(typed.selected().and_then(|command| command.aliases.first()), Some(&"card"));
    }

    #[test]
    fn a_slash_in_the_middle_of_a_word_is_prose() {
        assert!(slash("e/ou").is_none(), "a slash inside a word");
        assert!(slash("12/2026").is_none(), "a date");
    }

    #[test]
    fn a_slash_after_whitespace_opens_the_list() {
        assert!(slash("- /tab").is_some(), "inside a list item");
        assert!(slash("> /card").is_some(), "inside a callout");
        assert!(slash("Veja /table").is_some(), "after a word and a space");
    }

    #[test]
    fn a_space_after_the_name_closes_the_popup() {
        assert!(slash("/table ").is_none(), "the command was typed; the rest is prose");
        assert!(slash("/").is_some(), "a bare slash lists every command");
    }

    #[test]
    fn a_slash_inside_a_fenced_block_is_code() {
        assert!(slash("```\n/tab").is_none(), "inside the fence");
        assert!(slash("```\nx\n```\n/tab").is_some(), "the fence was closed");
        assert!(slash("~~~\n/tab").is_none(), "a tilde fence counts too");
    }

    #[test]
    fn the_word_being_typed_starts_after_the_last_space() {
        let typed = slash("Veja /tab").expect("a command after a space");
        assert_eq!(typed.range.start, 5, "the insertion replaces `/tab`, not the space before it");
    }

    #[test]
    fn the_contents_command_waits_for_a_heading() {
        assert!(for_caret("/toc", 4, false).is_none(), "nothing to list yet");
        assert!(for_caret("/toc", 4, true).is_some());
        assert!(for_caret("/h1", 3, false).is_some(), "the other commands do not need headings");
    }

    #[test]
    fn a_table_selects_its_first_header_cell() {
        let (text, selection) = take("/tab", "table");
        assert!(text.starts_with("| Col 1 | Col 2 | Col 3 |\n| --- | --- | --- |\n| Cell | Cell | Cell |\n"), "{text:?}");
        assert_eq!(&text[selection.start..selection.end], "Col 1", "the placeholder is selected, so typing replaces it");
    }

    #[test]
    fn the_command_is_replaced_and_the_rest_of_the_line_stays() {
        let (text, _) = take("antes\n/task", "task");
        assert_eq!(text, "antes\n- [ ] ");
    }

    #[test]
    fn a_block_taken_mid_line_gets_a_line_of_its_own() {
        let (text, selection) = take("Veja /table", "table");
        assert!(text.starts_with("Veja \n| Col 1 |"), "the table starts on its own line: {text:?}");
        assert_eq!(&text[selection.start..selection.end], "Col 1");
        // A snippet that is not a block stays where the caret is.
        let (text, _) = take("Veja /bold", "bold");
        assert_eq!(text, "Veja **text**");
    }

    #[test]
    fn bold_wraps_the_word_and_selects_it() {
        let (text, selection) = take("/bold", "bold");
        assert_eq!(text, "**text**");
        assert_eq!(&text[selection.start..selection.end], "text");
    }

    #[test]
    fn the_card_is_the_callout_the_review_panel_reads() {
        let (text, selection) = take("/card", "card");
        assert_eq!(text, "> [!card]- Question\n> Answer\n");
        assert_eq!(&text[selection.start..selection.end], "Question", "on the question, ready to be typed over");
    }

    #[test]
    fn the_contents_is_built_from_the_headings_nested_by_level() {
        let (text, selection) = take("/toc", "toc");
        assert_eq!(text, "- [[#Sepse]]\n  - [[#Conduta]]\n    - [[#Antibiótico]]\n");
        assert_eq!(selection, editor::Selection::caret(text.len()), "the caret ends after the list");
    }

    #[test]
    fn the_date_and_the_time_are_read_at_insertion_time() {
        let (today, _) = take("/date", "date");
        assert_eq!(today, "2026-11-17");
        let (now, _) = take("/time", "time");
        assert_eq!(now, "09:41");
    }

    #[test]
    fn the_highlight_wraps_around() {
        let mut typed = slash("/").expect("a command");
        let last = typed.matches.len() - 1;
        move_by(&mut typed, -1);
        assert_eq!(typed.index, last, "up from the first goes to the last");
        move_by(&mut typed, 1);
        assert_eq!(typed.index, 0);
    }

    #[test]
    fn every_command_inserts_something_and_a_consistent_selection() {
        // The catalogue is data: one bad row would be a popup entry that does nothing, or worse, a
        // selection out of bounds (a panic in the editor).
        for command in COMMANDS {
            let (text, selection) =
                insertion(command, &headings(), "2026-11-17", "09:41").unwrap_or_else(|| panic!("{} inserts nothing", command.id));
            assert!(!text.is_empty(), "{} is empty", command.id);
            assert!(selection.start <= selection.end, "{}: backwards selection", command.id);
            assert!(selection.end <= text.len(), "{}: selection past the text", command.id);
            assert!(!command.hint.is_empty(), "{} has no hint", command.id);
            assert!(!command.aliases.is_empty(), "{} has no name", command.id);
        }
    }

    #[test]
    fn every_command_is_reachable_by_its_own_name() {
        for command in COMMANDS {
            let name = command.aliases.first().copied().unwrap_or_default();
            let typed = for_caret(&format!("/{name}"), 1 + name.len(), true);
            assert!(typed.is_some(), "{} does not find itself by /{name}", command.id);
            assert!(typed.is_some_and(|typed| typed.matches.iter().any(|found| found.id == command.id)), "{} is not offered by /{name}", command.id);
        }
    }
}
