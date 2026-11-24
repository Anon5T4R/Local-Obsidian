<p align="center">
  <img src="assets/app-icon/hicolor/128x128/apps/com.localobsidian.app.png" width="96" height="96" alt="Local Obsidian icon">
</p>

<h1 align="center">Local Obsidian</h1>

<p align="center">
  Markdown notes in a plain folder, for Windows, Linux and Android. No account, no sync service, written in Rust.<br>
  <a href="https://github.com/Anon5T4R/Local-Obsidian/releases/latest">Download</a> ·
  <a href="ROADMAP.md">Roadmap and status</a> ·
  <a href="#where-it-comes-from">Where it comes from</a> ·
  <a href="#how-it-is-made">How it is made</a>
</p>

<!-- README-SHOT: a real vault, note in Split view (editor + reading), sidebar with tree and tags; dark and light -->
<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/readme/main-dark.png">
    <img src="docs/images/readme/main-light.png" width="820" alt="Local Obsidian with a note open in split view">
  </picture>
</p>

Local Obsidian is not affiliated with Obsidian or Dynalist Inc. It reads the same kind of vault (a
folder of Markdown files), but it is a separate app.

## How your notes are stored

A vault is a folder you already have. Notes are `.md` files with optional YAML frontmatter,
attachments go in `_attachments/`, and the only extra file the app writes is
`.openobsidian/srs.json` (flashcard review state). There is no database and no hidden copy: any
other editor or sync tool (OneDrive, Syncthing, git) sees exactly what the app sees, and changes made
outside are picked up while the app is open. Saves are atomic (a temporary file, then a rename), so a
crash never leaves half a note.

## What it does

- Notes with `#tags`, `[[wikilinks]]` (resolved to the nearest note), embeds (`![[Note#Section]]`),
  backlinks, aliases and templates.
- Edit, Read or Split view; the reading view draws headings, lists, tasks you can tick, quotes,
  callouts, tables, code and images.
- Search across the vault with `tag:`, `path:`, `file:`, quoted phrases, exclusions and regex.
- Slash commands (`/`) to insert blocks, an outline of the open note, daily notes.
- Flashcards from callouts and cloze deletions, reviewed with SM-2 scheduling; the Android app reviews
  the same cards.
- Import `.odt` and `.docx` as notes; export a note as HTML or as a PDF with real, selectable text.
- Optional AI chat and flashcard generation with your own provider.
- Interface in English, Portuguese, Spanish and Japanese; system, light or dark theme.

The full list, what it deliberately does not do (plugins, built-in PDF viewer, rendered math) and
why, is in the [roadmap](ROADMAP.md).

## Android

The Android app lives in [`android/`](android/) and opens the same vaults, through the system file
picker. It has its own release file in the same Releases page.

## Download

Windows 10/11 (x64): `.msi` installer or portable `.zip`. Linux (x86_64): `.AppImage`, `.deb`,
`.rpm`, Arch package or `.tar.gz`. Android: `.apk`. All in
[Releases](https://github.com/Anon5T4R/Local-Obsidian/releases). macOS is not published.

Each desktop file is signed with [minisign](https://jedisct1.github.io/minisign/), the same key as
every Local app:

```sh
minisign -Vm <file> -P RWTqN+hTTCFbDCGCA1cEJRLp2FSluxIUiJfqdoAYwkwNNVH5MTYH8qSN
```

## Network and privacy

- Your notes never leave the folder you chose.
- The desktop app looks for updates only when you ask (Help → About). On Android the update check is
  off until you turn it on.
- AI does nothing until you add a provider and your own key (any OpenAI-compatible service, or a
  local server such as llama.cpp or Ollama). Only the note or text you send goes to it.
- No account, analytics or crash reporting.

Security notes: the app's own code (seven crates and the app) has no `unsafe` blocks, the reading
view never loads anything from the internet, and releases are built by GitHub Actions from a tagged
commit with locked dependencies checked by `cargo-deny`.

## Where it comes from

Local Obsidian is a rewrite of **OpenObsidian**, an earlier app by the same maintainer built with
Electron, React and TypeScript. The old app is the specification of behaviour; the code is new, in
Rust, on the structure every Local app shares (it was started from our app template). Deliberate
differences from the Electron app are recorded in [UPSTREAM-SYNC.md](UPSTREAM-SYNC.md).

Why rewrite instead of keeping Electron: one runtime for the whole suite, a much smaller app that
starts fast, no web engine between your files and the screen, and the same shell, updater and
release process as the other Local apps. What it cost: Electron plugins cannot exist here, and some
features had to be rebuilt from scratch (PDF export, the reading view).

## How the app is built

| Crate | Role |
|---|---|
| `vault` | folder scan, note tree, the index (tags, aliases, backlinks), link resolution, safe writes |
| `md` | the note format: frontmatter, tags, wikilinks, embeds, the reading model |
| `editor` | editing commands as pure text transforms (wrap, list continuation, checkboxes) |
| `review` | spaced repetition and the shared `srs.json` |
| `convert` | `.odt` / `.docx` import |
| `pdf` | PDF export with real text |
| `ai` | the optional AI chat |

None of these crates knows about the UI or the file system beyond what its role needs;
`cargo xtask layers` enforces the direction. The desktop app (`apps/localobsidian`) puts them behind
an egui interface.

## Keyboard shortcuts

`Ctrl+Shift+F` search, `Ctrl+Shift+O` outline, `Ctrl+B` / `Ctrl+I` / `Ctrl+E` bold / italic / code,
`Ctrl+Shift+E` highlight, `Ctrl+Enter` checkbox, `Ctrl+S` save (it also autosaves),
`Ctrl+=` / `Ctrl+-` / `Ctrl+0` zoom. Help → Keyboard shortcuts and help lists them all, with the note syntax.

## Build from source

```sh
git clone https://github.com/Anon5T4R/Local-Obsidian
cd Local-Obsidian
cargo run --release -p localobsidian
cargo xtask ci        # all local checks
```

`cargo xtask qa-vault` builds a test vault of 580+ notes covering every block type, and
`cargo xtask qa-shots` screenshots the app on it at four window widths.

## How it is made

Local Obsidian follows the same rules as every Local app: local files and no account, Rust only
(including the UI and the cryptography), one self-contained repository per app, governed
dependencies, signed releases built from a tag, optional AI with your own provider, and MCP so
agents can use the app. Most of the code is written by AI coding agents (Claude Code and OpenCode)
from written plans, reviewed by the maintainer and held to the same checks and CI. The reasons for
each decision:
[How we make the Local apps](https://github.com/Styx-Studios/.github/blob/main/profile/README.md#how-we-make-the-local-apps).

## License

MIT or Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE));
third-party notices in [NOTICE](NOTICE) and asset credits in
[assets/ATTRIBUTION.md](assets/ATTRIBUTION.md). Other Local apps: [the full list](https://github.com/Styx-Studios).
