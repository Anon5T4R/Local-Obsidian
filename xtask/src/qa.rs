//! The local QA battery: a generated vault that holds every block a note can have, and the window
//! shots taken from it.
//!
//! The point is that the test **spec is real content**, not a demo: the size of the vault the app is
//! used on (500+ notes, 100+ tags, similar names), the block types a real note mixes, the odd cases
//! (a 20-row table, a cell with a paragraph inside, an emoji, a broken frontmatter) and the binaries
//! the tree keeps out. Nothing here ships: the vault lands in `target/qa-vault/`, and the shots in
//! `target/qa-f7/`.
//!
//! `qa-vault` writes it; `qa-shots` builds the app, opens that vault at four window sizes and keeps a
//! PNG of each. `qa-sample` copies a handful of notes from the vault the user actually has (never
//! writing to it) so the same shots can be taken over real material.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The note the shots are taken on: every block type in one place.
pub const KITCHEN_SINK: &str = "Bloqueios - tudo em um.md";

/// How many notes the generated vault holds: the size the app is used at.
const NOTES: usize = 520;

/// The note with one of every block the reading view knows. Committed in `qa/kitchen-sink.md`, so the
/// generator and the app's own layout test use the same file.
fn everything() -> Result<String, String> {
    let path = crate::root().join("qa").join("kitchen-sink.md");
    std::fs::read_to_string(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}
/// A broken frontmatter: the reading view must show the note, not the YAML error.
const BROKEN_FRONTMATTER: &str = "---\ntipo: [quebrado\ntags: [qa\n---\n\n# Frontmatter inválido\n\nA nota abre assim mesmo.\n";

/// `cargo xtask qa-vault [dir]`: writes the vault (default `target/qa-vault`).
pub fn vault(dir: Option<&str>) -> Result<(), String> {
    let root = crate::root();
    let target = dir.map_or_else(|| root.join("target").join("qa-vault"), PathBuf::from);
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|error| format!("cannot clear {}: {error}", target.display()))?;
    }
    std::fs::create_dir_all(&target).map_err(|error| format!("cannot create {}: {error}", target.display()))?;

    write(&target, KITCHEN_SINK, &everything()?)?;
    write(&target, "_qa/Frontmatter inválido.md", BROKEN_FRONTMATTER)?;
    write(&target, "_qa/Sem frontmatter.md", "# Sem frontmatter\n\nUma nota limpa.\n")?;
    write(&target, "_qa/Frontmatter vazio.md", "---\n---\n\n# Frontmatter vazio\n")?;
    write(&target, "_qa/Frontmatter cheio.md", "---\ntipo: qa\nnumero: 7\nbooleano: true\nlista: [a, b]\n---\n\n# Frontmatter cheio\n")?;

    // The bulk: numbered folders, similar names (the "closest note wins" rule) and tags in volume.
    for index in 1..=NOTES {
        let folder = match index % 4 {
            0 => "0. Sistema".to_string(),
            1 => "1. Dashboard".to_string(),
            2 => "2. Medicina/Ciências Básicas".to_string(),
            _ => "2. Medicina/Clínica Médica".to_string(),
        };
        let tag = format!("qa/grupo-{}", index % 40);
        let neighbour = index % NOTES + 1;
        let body = format!(
            "---\ntipo: qa\n---\n\n# Nota similar {index}\n\n#qa #tipo-qa #{tag} #volume\n\n\
             Uma nota de volume. Ver [[Nota similar {neighbour}]] e [[Nota similar {}]].\n\n\
             ## Seção\n\nTexto da seção, com **ênfase** e `código`.\n",
            (index + 7) % NOTES + 1
        );
        write(&target, &format!("{folder}/Nota similar {index}.md"), &body)?;
    }

    // Tags in volume: 100+ across the notes, deliberately similar.
    for index in 0..60 {
        let body = format!("# Tag {index}\n\n#qa #tag-{index} #tag-{} #sistema/grupo-{}\n", index % 60 + 1, index % 12);
        write(&target, &format!("_qa/tags/Tag {index}.md"), &body)?;
    }

    // An image the embeds can point at, and the binaries the tree keeps out.
    std::fs::write(target.join("figura-qa.png"), PNG).map_err(|error| format!("cannot write the image: {error}"))?;
    write(&target, "leia-me.txt", "Um arquivo de texto: não é nota, e a árvore não o lista.\n")?;
    std::fs::write(target.join("protocolo.pdf"), PDF).map_err(|error| format!("cannot write the pdf: {error}"))?;
    write(&target, "_templates/daily.md", "# {{title}}\n\n{{date}}\n")?;

    let notes = count_notes(&target);
    println!("qa-vault: {} ({notes} notes)", target.display());
    Ok(())
}

/// `cargo xtask qa-shots [vault]`: one PNG of the app per window size.
pub fn shots(vault: Option<&str>) -> Result<(), String> {
    let root = crate::root();
    let app = root.join("target").join("debug").join(if cfg!(windows) { "localobsidian.exe" } else { "localobsidian" });
    if !app.is_file() {
        return Err(format!("{} is missing: run `cargo build` first", app.display()));
    }
    let vaults: Vec<PathBuf> = match vault {
        Some(dir) => vec![PathBuf::from(dir)],
        None => {
            let generated = root.join("target").join("qa-vault");
            if !generated.is_dir() {
                return Err(format!("{} is missing: run `cargo xtask qa-vault` first", generated.display()));
            }
            // The real sample rides along when it exists.
            let real = root.join("target").join("qa-vault-real");
            let mut all = vec![generated];
            if real.is_dir() {
                all.push(real);
            }
            all
        }
    };
    let shots = root.join("target").join("qa-f7");
    std::fs::create_dir_all(&shots).map_err(|error| format!("cannot create {}: {error}", shots.display()))?;

    let started = std::time::Instant::now();
    let mut written = 0;
    for vault in &vaults {
        let label = if vault.ends_with("qa-vault-real") { "real" } else { "gerado" };
        // The note to open: the kitchen sink in the generated vault, a representative note (when the
        // sample has it) in the real one; otherwise the first note of the vault.
        let wanted = if label == "real" { "Análise de Lacunas do Vault" } else { "Bloqueios - tudo em um" };
        let note = if vault.join(format!("{wanted}.md")).is_file() { Some(wanted.to_string()) } else { None };
        for (width, height) in [(1920, 1080), (1366, 768), (1024, 600), (800, 500)] {
            let shot = shots.join(format!("vault-{label}-{width}x{height}.png"));
            let mut command = Command::new(&app);
            command
                .env("LOCALOBSIDIAN_NO_PREFS", "1")
                .env("LOCALOBSIDIAN_QA_SHOT", &shot)
                .env("LOCALOBSIDIAN_QA_VAULT", vault)
                .env("LOCALOBSIDIAN_QA_VIEW", "read")
                .env("LOCALOBSIDIAN_QA_SIZE", format!("{width}x{height}"))
                .env("LOCALOBSIDIAN_QA_FRAMES", "45");
            if let Some(note) = &note {
                command.env("LOCALOBSIDIAN_QA_NOTE", note);
            }
            let status = command.status().map_err(|error| format!("cannot start {}: {error}", app.display()))?;
            if !status.success() {
                return Err(format!("{} exited with {status}", app.display()));
            }
            if !shot.is_file() {
                return Err(format!("{} was not written", shot.display()));
            }
            written += 1;
        }
    }
    let elapsed = started.elapsed();
    println!("qa-shots: {written} PNGs in {} ({:.1}s)", shots.display(), elapsed.as_secs_f32());
    if elapsed.as_secs() > 120 {
        println!("qa-shots: slower than two minutes - keep it out of CI and run it at the end of a phase");
    }
    Ok(())
}

/// `cargo xtask qa-sample [n]`: copies notes from the vault the user actually has, read-only, into
/// `target/qa-vault-real/`. The original is never written to.
pub fn sample(count: Option<&str>) -> Result<(), String> {
    let root = crate::root();
    let wanted: usize = count.and_then(|count| count.parse().ok()).unwrap_or(20);
    let prefs = prefs_path().ok_or("cannot find the app's settings file on this machine")?;
    let text = std::fs::read_to_string(&prefs).map_err(|error| format!("cannot read {}: {error}", prefs.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| format!("{} is not JSON: {error}", prefs.display()))?;
    let vault =
        value.get("last_vault").and_then(|path| path.as_str()).filter(|path| !path.is_empty()).ok_or("the settings file has no last_vault")?;
    let source = PathBuf::from(vault);
    if !source.is_dir() {
        return Err(format!("{vault} is not a folder"));
    }
    let target = root.join("target").join("qa-vault-real");
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|error| format!("cannot clear {}: {error}", target.display()))?;
    }
    std::fs::create_dir_all(&target).map_err(|error| format!("cannot create {}: {error}", target.display()))?;

    // Every Nth note of the tree: spread over the folders instead of the first ten alphabetically.
    let mut notes: Vec<PathBuf> =
        walk(&source).into_iter().filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("md"))).collect();
    notes.sort();
    if notes.is_empty() {
        return Err(format!("{vault} has no notes"));
    }
    let step = (notes.len() / wanted).max(1);
    let mut copied = 0;
    for note in notes.iter().step_by(step).take(wanted) {
        let relative = note.strip_prefix(&source).unwrap_or(note);
        let destination = target.join(relative);
        if let Some(folder) = destination.parent() {
            std::fs::create_dir_all(folder).map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
        }
        // A copy, never a move or a write at the source.
        std::fs::copy(note, &destination).map_err(|error| format!("cannot copy {}: {error}", note.display()))?;
        copied += 1;
    }
    println!("qa-sample: {copied} notes from {vault} into {}", target.display());
    Ok(())
}

/// The settings file, per platform (see `apps/localobsidian/src/prefs.rs`).
fn prefs_path() -> Option<PathBuf> {
    let dir = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)?.join("LocalObsidian")
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/LocalObsidian")
    } else {
        match std::env::var_os("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
        }
        .join("localobsidian")
    };
    Some(dir.join("prefs.json"))
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        let hidden = path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.'));
        if hidden {
            continue;
        }
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn write(target: &Path, relative: &str, content: &str) -> Result<(), String> {
    let path = target.join(relative);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
    }
    std::fs::write(&path, content).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

fn count_notes(dir: &Path) -> usize {
    walk(dir).iter().filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("md"))).count()
}

/// A 1×1 PNG: something for the embeds and the image path to point at.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// The smallest well-formed PDF: a page with nothing on it. The tree must keep it out.
const PDF: &[u8] = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";
