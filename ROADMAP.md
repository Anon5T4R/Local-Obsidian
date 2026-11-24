# Local Notebook roadmap

The honest state of the app: what works today, what it deliberately does not do, and what is planned. Release notes
list what changed in each version.

## What works today

- **Vault**: open/choose the folder (the last vault comes back on the next start), tree with notes before folders
  and sorting by name or modification, instant filter by name/tag/alias, **tag chips** with counts and
  **backlinks** for the open note. The sidebar is a single one, with three blocks that each have their own height:
  the tree, the tags section (closed by default, with its own scrolling) and the backlinks — none of them pushes or
  covers the others.
- **Note**: YAML frontmatter read and preserved, `#tags` (including nested ones) and `[[wikilinks]]` with
  resolution "the closest note wins" and click navigation (including to the section); `![[Note#Section]]`
  transcludes; **`[[` autocomplete** while typing.
- **Search** (`Ctrl+Shift+F`): panel with `tag:`/`path:`/`file:`, `"phrases"`, `-exclusions` and `.*` mode, results
  per note (most relevant first) with the matching lines and their number; clicking opens the note at the line.
- **Outline** (`Ctrl+Shift+O`): headings of the open note; clicking scrolls to the section.
- **Editor**: plain text with `Ctrl+B`/`I`/`E` (bold/italic/code), `Ctrl+Shift+E` (highlight), `Tab`/`Shift+Tab`,
  `Ctrl+Enter` (checkbox), list continues on Enter, 800 ms autosave and `Ctrl+S`; atomic writes (temporary file +
  rename), never half a file.
- **Slash commands**: `/` at the start of a word (start of line or after a space, outside a code block) opens the
  list of what can be inserted — headings `h1`/`h2`/`h3`, `bold`, `italic`, `strike`, `highlight`, `inline`,
  `comment`, `list`, `numlist`, `task`, `table`, `code`, `quote`, `hr`, `wikilink`, `embed`, `link`, `image`,
  `card`, `cloze`, `toc` (table of contents built from the note's headings), `callout`/`tip`/`warning`/`danger`,
  `frontmatter`, `date` and `time`. `↑`/`↓` move, `Enter`/`Tab` accept, `Esc` dismisses; the example snippet comes
  **selected**, so typing replaces it, and a block taken mid-line gets its own line first.
- **Reading**: headings, lists, quote, callouts (`> [!warning]`), aligned tables, code blocks, images from
  `_attachments/` and clickable links; **Edit | Split | Read** in a selector always visible in the note header
  (Split has a draggable divider, saved in preferences). A `- [ ]` in reading mode is a real checkbox: clicking it
  toggles the line in the file. `#tag` in the body becomes a chip, and clicking it filters the sidebar. Tables use
  the page width, with a header band, alternating rows and word wrapping.
- **Book-style reading**: the column is 820 px and centered, with real type steps (H1 29, H2 21, body 15) and
  rhythm between blocks — text no longer becomes a wall of 150 characters per line. The frontmatter shows as a
  **properties card** at the top (type, aliases and tags as chips, in file order), inline code is a neutral chip,
  the table has ~40 px of breathing room per row, and the code panel in Split gets highlighting (fences, headings,
  quotes, `#tags`, `[[links]]`, `==highlights==`). **Rendering zoom** from 0.8 to 1.6 (`Ctrl+=`, `Ctrl+-`, `Ctrl+0`
  or `View → Zoom`), saved in preferences.
- **Folder documents**: `.pdf`, `.docx`, `.odt`, `.epub` and `.txt` files in the vault show in the tree — deleted
  ones, between notes and folders — and **open in the system app** with a click; the context menu of a folder (or
  of a document) shows it in the file manager. They still are not notes: search, wikilinks and backlinks do not see
  them.
- **AI** (`AI → Chat about the note`): chat panel that sends the open note as context and receives the answer in
  chunks (SSE), with `Send`, `Stop`, `Insert into note` and `Clear`. The connection (`AI → Connection…`) accepts
  any provider compatible with the OpenAI API — Ollama and llama-server are a base URL and a model —, fetches the
  provider's model list, and stores the key **in the system keychain**, never in a file. Without a connection, the
  panel says what is missing. It is the suite's shared `local-ai`, with no server of ours in the path: nothing
  leaves the machine without the endpoint the user configured.
- **Import document** (`File → Import a Word or OpenDocument file...`): a `.docx` or `.odt` becomes a note in the
  folder of the open note, with headings, lists (numbered ones included), bold, italic, links and tables. Images,
  charts, footnotes and comments are left out — the same cut the source app makes. The `.docx` is read directly
  from the package (ZIP + XML), without Office installed.
- **Import Anki cards** (`File → Import Anki cards (text)...`): a text export (`.txt`, `.tsv`, `.csv`) becomes `>
  [!card]` notes, 100 cards per note, with the deck's tags at the top. Media from an `.apkg` does not come in (see
  the table below).
- **Rename** a note rewrites the `[[links]]` that pointed to it (and says how many).
- **Review (SRS)**: flashcards written in the note itself — `> [!card]- Question` with the answer in the body, or
  `> [!card]` with `==highlights==` (one gap per highlight, cloze-style) and `> [!mnemonic]?` — scheduled by SM-2
  and reviewed in a keyboard-only panel (`Space` shows, `1`–`4` grade, `Esc` closes). Decks: whole vault, the open
  note, or a tag, **chosen in the panel itself** (the selector lists only what has cards, with the count of each),
  and the header states the scope and its numbers (cards in the deck · due today · done in the session). Cards
  render the note's Markdown; the statistics panel follows the session's deck, with retention, average ease, a
  14-day forecast and the queue of upcoming cards with their source note. The schedule lives in
  `.openobsidian/srs.json`, **the same file the Android app reads** (same fields, same order, no extra state);
  the card stores a stable id (hash of the question), so editing the answer does not reset the scheduling.
- **Query blocks**: a ` ```query ` fence is not code to read, it is a question to answer — `tag:`, `path:`, `has:`
  and any frontmatter field (`tipo: patologia`), with `sort: titulo|modificado|caminho [desc]` and `limit:`.
  Reading mode shows "N results" and the clickable list (the matching notes, in the requested order), with `+N
  more` to reveal the rest; a line the grammar does not understand appears as a warning instead of being ignored.
  Same grammar as the Android app.
- **AI flashcards** (`AI → Create flashcards from this note`, `Ctrl+Shift+K` or the note's menu in the tree): the
  model writes `> [!card] pergunta` + `> ==resposta==` from the open note and the result goes to **a new note next
  to it** (`<note> AI.md`), with the frontmatter and tags inherited plus `flashcards-ai`; the original note is not
  touched and undo deletes only the created one. The dialog chooses cards per section (1/2/3/5) and language, shows
  the stream with Stop, and reopens with the reason if the engine fails.
- **Vault diagnostics** (`View → Vault diagnostics`): links that point to a nonexistent note (with who wrote them),
  orphan notes (no incoming or outgoing link) and duplicate names in different folders — each line opens the note.
- **Connections** (`View → Show the connections`): the open note in the center and its direct connections (outgoing and
  incoming links) in rings per hop, with each note's `+` opening *its* connections — the map grows only through the
  part of the vault that matters. Clicking a note opens the note; the menu on the right opens, collapses or starts
  the map from there; the bar has "Collapse all" and "Start from the open note". Deterministic layout (no physics):
  the same vault draws the same map.
- **Calendar** (`View → Show the calendar`): month grid with the days that already have notes marked; clicking opens
  the daily note (`YYYY-MM-DD`) and creates it when missing, from `_templates/daily.md` if it exists.
- **Templates**: when creating a note, choose one of the four built-in ones (blank, daily, meeting, project) or one
  of your own from `_templates/`, with `{{title}}`, `{{date}}`, `{{date:DD/MM/YYYY}}`, `{{time}}` and
  `{{datetime}}` expanded.
- **Watched vault**: changes made by another program (editor, sync client) come in on their own, with a 300 ms
  debounce; the app's own writes and `srs.json` are ignored. If the open note has unsaved text, **your text wins**
  and the status bar warns.
- **Export HTML** (`File → Export the note as HTML…`): the open note becomes a static page, with embedded CSS and the
  palette following the system theme of whoever opens it. `[[Note]]` becomes `Note.html` (relative), callouts
  become boxes colored by type, tables get a header and alignment, and the images the note shows are copied to an
  `_attachments/` next to the page.
- **Export PDF** (`File → Export the note as PDF…`): the note becomes **A4 pages with real text** — the glyphs go
  with the embedded fonts (subset) and the Unicode map, so selecting, searching and copying work in the PDF reader.
  What the reading view draws comes out: headings, paragraphs with bold/italic/strikethrough/code/highlight, lists
  and tasks with sub-lists indented per level, quote, callout with the type's color, table with grid and measured
  columns, monospaced code, the vault's images embedded (up to 16 MB and 64 MP — the size comes from the file
  header, so a giant PNG becomes `[image: …]` without ever being decoded; without the file, or in a format the
  writer does not read, the same marker stays) and the page number. **Links** to the web become clickable areas in
  the reader; a `[[wikilink]]` stays text, because it only exists inside the app.
- **Help** (`Help → Keyboard shortcuts and help`): the keys, the syntax a note accepts, and where each thing lives in the
  vault, with a link to this documentation.
- Interface in pt-br, es and ja.
- **Theme** (`View → Theme`): **System** (follows the desktop's light/dark, the default), **Light** or **Dark**,
  saved in preferences. Only the accent is ours (the violet of the icon, darkened on white and lightened on dark,
  for link, tag and quote); the rest of the gray is the theme's — and callouts, which have their own color, darken
  in light mode so the title stays readable.

## What it does not do, and why

| What | Why |
|---|---|
| **Plugins / extension system** | The source app is Electron: the "plugin" was JS code running in the renderer. Here there is no renderer, no JS, no dynamic loader — it is a Rust binary. Not a pending item; a structural divergence. |
| **In-app viewer for PDF/EPUB/DOCX** | Documents show in the tree and open in the system reader — a PDF is not a note and the app does not try to render it. A `.docx`/`.odt` that needs to become text comes in through **import**, which creates an editable note. |
| **Import of `.apkg` (Anki)** | Reading the package requires a SQLite engine. The only mature 100% Rust option today (`turso`/`limbo`) is still pre-release and brings a large tree; `rusqlite` compiles C, which the port does not accept. The Anki **text** import (`File → Import Anki cards (text)...`) covers real use, and the `.apkg` is annotated with this path. |
| **Rendered math** | Stays in its own unit (needs math fonts + a painter). For now TeX shows cleanly in monospace, without turning into a soup of symbols. |
| **Built-in AI model** | The app is a client: it talks to the OpenAI-API-compatible endpoint the user configures (Ollama, llama-server, remote provider). It does not ship weights or run inference — that is the job of `local-ai`, which is just the suite's shared client. |
| **Emoji font** | A maintainer decision: no emoji font is shipped. **Emoji is never drawn** in reading mode (a monochrome pictogram in the middle of text reads as an error), and symbol ranges — arrows, math operators — show up when the suite's fonts have the glyph. The note's source text does not change, and the editor shows the character as it is. |

## Planned

- **Import of `.apkg`** once there is a mature pure-Rust SQLite reader (or a b-tree reader of our own).
- **Rendered math** (fonts + painter).

## Android

The Android app lives in `android/` and opens the same vaults (files only). Its state is in `android/README.md`.
