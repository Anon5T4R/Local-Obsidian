# AGENTS.md — Local Obsidian

App desktop da suíte Local: **Rust puro + egui forkado**, sem webview/React/TS. O vault é a pasta
do usuário (arquivos `.md` + frontmatter + wikilinks + `_attachments/`); o único estado próprio é
`.openobsidian/srs.json`. Sem conta, telemetria ou serviço de terceiro.

Comportamento vem do **OpenObsidian** (`C:\Dev\OpenObsidian\desktop`, Electron v1.2.1) como spec;
estrutura vem do padrão craft (`docs/planos/padrao-craft.md` no dev-notes). Contrato de vault
compartilhado com o OpenObsidianAndroid: files-only (backlog B18).

## Regras
- **Dependências:** toda dep que entra/sai/atualiza exige `Cargo.lock` commitado + linha no
  inventário do repo `Anon5T4R/local-deps` (regra completa em `local-deps/WORKFLOW.md`).
- **Sem fontes git:** nenhuma dependência pode apontar para `git+` (o CI valida; vale para o
  `Cargo.lock` da raiz e o de `libs/craft-libs`). Tudo vem do crates.io (pinado pelo lock) ou de
  `libs/` (vendored). Dep nova também precisa ser pure Rust (sem C) e compatível com o egui forkado
  via `[patch]`.
- **egui/winit/craft-libs do fork vendored**, nunca do crates.io; nunca `lab-ui`.
- **craft-update tem fonte única** (`Anon5T4R/local-libs`): a cópia em `libs/craft-libs/crates/update`
  é espelho — mudança se faz na fonte e replica com `scripts/sync.ps1 -Mode push`.
- **Never-crash:** nada de `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!` fora de testes (o clippy
  barra), `unsafe` proibido. Toda entrada (caminho, arquivo, argumento) é hostil até prova em
  contrário — e num vault, arquivo de terceiro é entrada hostil por definição.
- **Notas são do usuário:** nada de formato proprietário, nada de reescrever o vault para caber no
  app. Save é atômico (temp + rename), rename preserva `[[links]]`.
- **Commit:** mensagem em português, sem Co-Authored-By, paths explícitos.
- **Release:** `cargo xtask version set X.Y.Z` **antes** de toda tag → tag `vX.Y.Z` → o Actions
  builda e publica. Mínimo local.
- **Disco:** no workspace do João vale a política de `target/` do dev-notes (`MEMORY.md` §4 — limpa o que
  está frio ou feito, mantém durante iteração ativa); fora dele, `cargo clean` ao terminar. Builds pesados são do CI.

## Estrutura
- `apps/localobsidian` — o app: `main.rs` (args, ícone, janela), `app.rs` (barra de menus, botões da suíte —
  paleta, tema, engrenagem —, Configurações, About com update, a página; a paleta de comandos fica em
  `app/commands.rs`), `i18n.rs`, `pickers.rs`, `fonts.rs`, `suite_entry.rs` (entrada no catálogo),
  `build.rs` (ícone/VERSIONINFO, catálogos i18n, fontes)
- `crates/localobsidian-*` — camadas (registradas em `xtask/src/layers.rs` `TABLE`); UI só em L5+
- `libs/` — forks vendored: `craft-libs` (com `crates/update` espelhado do `local-libs`),
  `egui` 6d518fc, `winit` a517b38, `winprint.rs`, `rustls-rustcrypto`
- `assets/` — `app-icon/`, `craft-fonts/` vendored (build input `CRAFT_FONTS_DIR`), `ATTRIBUTION.md`
- `i18n/locales/` — catálogos TSV `context⇥fonte EN⇥tradução`, embutidos pelo `build.rs`
- `packaging/` — Windows (WiX/MSI), Linux (AppImage/deb/rpm/tar), Arch (PKGBUILD), minisign
- `xtask/` — `ci`, `fmt`, `assets`, `layers`, `version`, `ico`
- `scripts/check-mirror.sh` — confere as crates e os assets (`craft-fonts`) espelhados do `local-libs` contra a fonte (CI e `xtask ci`)

## Build
```
cargo build --locked          # debug
cargo test --locked           # as fontes vendored entram sempre (.cargo/config.toml), igual ao CI
cargo xtask ci                # o gate local: o mesmo que o CI (fmt, clippy, build, assets, layers,
                              # testes, cargo-deny, espelhos do local-libs)
packaging/windows/package.ps1 # MSI + portable (Windows)
packaging/linux/package.sh    # AppImage/deb/rpm/tar (Linux)
```

## Assets
- Nenhum asset de produto proprietário. Só trabalho próprio, domínio público/CC0, CC-BY com
  atribuição, OFL ou permissivo (MIT/Apache).
- Todo asset tem linha em `assets/ATTRIBUTION.md` no mesmo commit (`cargo xtask assets` valida;
  `libs/` é árvore vendored e carrega a atribuição de origem).
- Fontes fora do que o egui embute vivem no craft-fonts (`assets/craft-fonts`).

## Upstream e a pasta `C:\Dev\storytold`

Menções a `storytold/*` em código, comentários, docs e atribuições são referência histórica e
obrigação de licença. **Nada** é lido de `C:\Dev\storytold` em build, teste ou release: tudo vem do
próprio repo (crates.io pinado pelo lock + `libs/`/`assets/craft-fonts` vendored). O spec funcional
do port vive em `C:\Dev\OpenObsidian\desktop` e é **somente leitura**.
