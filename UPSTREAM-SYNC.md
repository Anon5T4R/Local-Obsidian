# UPSTREAM-SYNC - Local Obsidian

O que neste repo vem de fora, em que revisão, e o que fazer quando o upstream mudar.

## De onde saiu este app

A árvore nasceu de **`Anon5T4R/local-app-template`** (`main` em `55a63ae`) via
`scripts/nascer-app.ps1 -Id localobsidian -Name "Local Obsidian" -Repo Local-Obsidian`, que renomeou
crate/pacote/binário/ids/env vars/pastas e gerou GUIDs novos do MSI. O ícone nasceu no próprio repo
(ver `assets/app-icon/README.md`).

O **comportamento** portado vem do **OpenObsidian** (`C:\Dev\OpenObsidian\desktop`, Electron + React
+ TS, v1.2.1): o TypeScript é spec, não estrutura. O que já foi portado e as divergências
conscientes ficam registrados aqui, seção a seção, conforme o port avança.

## Spec funcional (somente leitura)

| O que | Onde | Regra |
|---|---|---|
| Comportamento (tree sort, filtro da sidebar, frontmatter, wikilinks/embeds, safe-write, link-rewrite) | `C:\Dev\OpenObsidian\desktop\src` | portar comportamento; nunca copiar estrutura React/Electron |
| Decisões travadas (11 itens com veredito) | `docs/limitations.md` do upstream | itens "Keep" são requisito: não "consertar" no port |
| Gramática de query | `utils/noteQuery.ts` + `NoteQuery` do Android | **uma só gramática**, igual ao Android de propósito |
| Contrato de vault | backlog B18 (dev-notes) | files-only: `.md` + frontmatter + wikilinks + embeds, `_attachments/`, `.openobsidian/srs.json` como único estado compartilhado |

## Forks vendored (`libs/`)

| Árvore | Origem | Revisão | Sync |
|---|---|---|---|
| `libs/egui` | storytold/egui (fork do egui) | `6d518fc799681571963842f95b55a6b68431b15a` | skill `sync-upstream`; patches da suíte (bidi, scripting, viewport hooks) |
| `libs/winit` | storytold/winit (fork do winit) | `a517b3846ee8457d84ee4975110bfda6797c44dc` | idem |
| `libs/craft-libs` | storytold/craft-libs | base `1e6f1278` | idem, **exceto** `crates/update` (abaixo) |
| `libs/craft-libs/crates/update` | **`Anon5T4R/local-libs`** | `HASHES.update` da fonte | **fora do sync-upstream**: fork permanente consciente. Mudança vai na fonte e replica com `scripts/sync.ps1 -Mode push`; a cópia aqui é espelho e nunca se edita in place |
| `libs/winprint.rs` | alampy/winprint.rs (BSD-3) | `417ac3c` (+ `winprint-gdi` no rev antigo) | usado por `craft-libs/crates/print`; upstream vivo, patches genéricos vão por PR |
| `libs/rustls-rustcrypto` | fork da suíte | - | o `craft-update` pina este fork (o upstream alpha puxa rustls-webpki 0.102 só por `alg_id`) |
| `assets/craft-fonts` | storytold/craft-fonts | `b076417b5880bd4154bd22f00695b1d28c6bea83` | build input (`CRAFT_FONTS_DIR`); atualiza trocando a árvore inteira e o rev em `packaging/fonts.sh`, `packaging/windows/fonts.ps1` e no CI |

Regra do repo: nada é lido de `C:\Dev\storytold` em build, teste ou release. As menções a
`storytold/*` em código, docs e atribuições são referência histórica e obrigação de licença.

## Catálogo da suíte

`craft_update::suite` (na crate espelhada) é a fonte única dos `App` de cada app. A entrada
`LOCALOBSIDIAN` foi adicionada em `Anon5T4R/local-libs` (`crates/update/src/suite.rs`) e replicada
para os apps com `scripts/sync.ps1 -Mode push`.
