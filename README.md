# Local Obsidian

Notas, wikilinks e anexos numa pasta comum — app desktop da suíte Local, em **Rust puro + egui
forkado** (sem webview, sem React, sem servidor). Um vault é a pasta que o usuário já tem:
`.md` com frontmatter YAML, `_attachments/` para binários e `.openobsidian/srs.json` como único
estado compartilhado. Nada de conta, telemetria ou serviço de terceiro.

Port do **OpenObsidian** para a família craft: o comportamento vem do app Electron (v1.2.1), a
estrutura é do padrão craft. O contrato de vault é o mesmo do OpenObsidianAndroid (files-only), para
que o mesmo cofre abra nos dois.

## O que funciona hoje (v0.4.0)

- **Cofre**: abrir/escolher a pasta (o último cofre volta no próximo start), árvore com notas antes
  das pastas e ordenação por nome ou modificação, filtro instantâneo por nome/tag/alias, **chips de
  tag** com contagem e **backlinks** da nota aberta.
- **Nota**: frontmatter YAML lido e preservado, `#tags` (inclusive aninhadas) e `[[wikilinks]]` com
  resolução "a nota mais próxima vence" e navegação por clique (inclusive até a seção); `![[Nota#Seção]]`
  transclui; **autocomplete de `[[`** enquanto digita.
- **Busca** (`Ctrl+Shift+F`): painel com `tag:`/`path:`/`file:`, `"frases"`, `-exclusões` e modo `.*`,
  resultados por nota (mais relevante primeiro) com as linhas e o número delas; clicar abre a nota na
  linha.
- **Outline** (`Ctrl+Shift+O`): títulos da nota aberta, clique rola até a seção.
- **Editor**: texto puro com `Ctrl+B`/`I`/`E` (negrito/itálico/código), `Ctrl+Shift+E` (destaque),
  `Tab`/`Shift+Tab`, `Ctrl+Enter` (checkbox), lista que continua no Enter, autosave de 800 ms e
  `Ctrl+S`; escrita atômica (arquivo temporário + rename), nunca meio arquivo.
- **Leitura**: títulos, listas, citação, callouts (`> [!warning]`), tabelas alinhadas, blocos de
  código, imagens de `_attachments/` e links clicáveis; **Edit | Read | Split**.
- **Renomear** uma nota reescreve os `[[links]]` que apontavam para ela (e diz quantos).
- **Revisão (SRS)**: flashcards escritos na própria nota — `> [!card]- Pergunta` com a resposta no
  corpo, ou `> [!card]` com `==destaques==` (uma lacuna por destaque, tipo cloze) e `> [!mnemonic]?` —
  agendados por SM-2 e revisados num painel só de teclado (`Espaço` mostra, `1`–`4` avaliam, `Esc`
  fecha). Baralhos: cofre inteiro, a nota aberta ou uma tag. Painel de estatísticas com vencimento,
  retenção, facilidade média e previsão de 14 dias. O cronograma mora em `.openobsidian/srs.json`,
  **o mesmo arquivo que o OpenObsidianAndroid lê** (mesmos campos, mesma ordem, nada de estado extra);
  o cartão guarda o id estável (hash da pergunta), então editar a resposta não zera o agendamento.
- **Grafo** (`Visualizar → Mostrar o grafo`): as notas como nós e os `[[links]]` como arestas, com a
  nota mais ligada no centro e o resto em anel; passar o mouse acende os links do nó e clicar abre a nota.
- **Calendário** (`Visualizar → Mostrar o calendário`): grade do mês com os dias que já têm nota
  marcados; clicar abre a nota diária (`YYYY-MM-DD`) e a cria quando falta, a partir de
  `_templates/daily.md` se existir.
- **Modelos**: ao criar uma nota, escolher um dos quatro embutidos (em branco, diária, reunião,
  projeto) ou uma nota sua de `_templates/`, com `{{title}}`, `{{date}}`, `{{date:DD/MM/YYYY}}`,
  `{{time}}` e `{{datetime}}` expandidos.
- **Cofre observado**: mudança feita por outro programa (editor, cliente de sync) entra sozinha, com
  debounce de 300 ms; as escritas do próprio app e o `srs.json` são ignorados. Se a nota aberta tiver
  texto não salvo, **o seu texto vence** e a barra de status avisa.
- Interface em pt-br, es e ja.

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
