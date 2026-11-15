# Local Obsidian

Notas, wikilinks e anexos numa pasta comum — app desktop da suíte Local, em **Rust puro + egui
forkado** (sem webview, sem React, sem servidor). Um vault é a pasta que o usuário já tem:
`.md` com frontmatter YAML, `_attachments/` para binários e `.openobsidian/srs.json` como único
estado compartilhado. Nada de conta, telemetria ou serviço de terceiro.

Port do **OpenObsidian** para a família craft: o comportamento vem do app Electron (v1.2.1), a
estrutura é do padrão craft. O contrato de vault é o mesmo do OpenObsidianAndroid (files-only), para
que o mesmo cofre abra nos dois.

## O que já vem do molde

| Peça | Onde |
|---|---|
| Janela egui com o frame certo por desktop (probe + `craft-window`), barra de menus | `apps/localobsidian/src/app.rs` |
| Janela **About** com verificação de atualizações (`craft-update`) e self-update por MSI/AppImage/pacman | `apps/localobsidian/src/app.rs`, `src/suite_entry.rs` |
| i18n por catálogo TSV (`craft-i18n`): pt-br, es, ja | `i18n/locales/`, `apps/localobsidian/src/i18n.rs`, `build.rs` |
| Seletor nativo de arquivo assíncrono (rfd 0.17 + pollster, sem travar o frame) | `apps/localobsidian/src/pickers.rs` |
| Fontes craft-fonts embutidas (CJK/árabe/hebraico/devanágari) | `assets/craft-fonts/`, `apps/localobsidian/src/fonts.rs`, `build.rs` |
| CI (ubuntu + windows + Arch + guardião de `git+`) e release por tag com minisign | `.github/workflows/` |
| xtask: `fmt`, `assets`, `layers`, `version`, `ico` | `xtask/` |

## Estrutura

```
apps/localobsidian/  app: main.rs (args, ícone, janela), app.rs (menus/About/página), i18n.rs,
                     pickers.rs, fonts.rs, suite_entry.rs, build.rs (ícone, i18n, fontes)
crates/              camadas `localobsidian-*` (registradas em xtask/src/layers.rs TABLE)
assets/              app-icon/, craft-fonts/ (vendored), ATTRIBUTION.md
i18n/locales/        catálogos de tradução (context ⇥ fonte EN ⇥ tradução)
libs/                forks vendored: craft-libs, egui, winit, rustls-rustcrypto, winprint.rs
packaging/           env.sh, fonts.sh, icons.sh, minisign.pub, linux/, arch/, windows/
xtask/               fmt, assets, layers, version, ico
```

Camadas: crie `crates/localobsidian-<nome>` e registre em `xtask/src/layers.rs` (`TABLE`) — o gate
`cargo xtask layers` cobra o registro e as direções (nada abaixo de L5 puxa egui).

## Build e verificação

```
cargo build --locked                  # debug (CRAFT_FONTS_DIR=assets/craft-fonts para as fontes CJK)
cargo test --locked                   # testes
cargo xtask fmt                       # fmt --check só dos membros (libs/ fica como veio)
cargo xtask assets                    # todo asset tem linha em assets/ATTRIBUTION.md
cargo xtask layers                    # camadas de crate
packaging/windows/package.ps1         # MSI + portable (Windows; precisa de WiX v5)
packaging/linux/package.sh            # AppImage/deb/rpm/tar (Linux; precisa de nfpm/appimagetool)
```

O CI roda exatamente `cargo xtask fmt`, clippy (`-D warnings`), `cargo build --locked`, os dois
gates, os testes (Linux) e o `makepkg --printsrcinfo` (Arch). Não deixe `target/` para trás:
`cargo clean` após uso local.

## Release

`cargo xtask version set X.Y.Z` **antes** da tag → `git tag vX.Y.Z` → `git push origin vX.Y.Z`. O
Actions builda, assina os assets com minisign e publica.

## Licença

MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`); licenças de terceiros no `NOTICE`.
