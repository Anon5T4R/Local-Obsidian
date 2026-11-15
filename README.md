# Local Obsidian

Notas, wikilinks e anexos numa pasta comum — app desktop da suíte Local, em **Rust puro + egui
forkado** (sem webview, sem React, sem servidor). Um vault é a pasta que o usuário já tem:
`.md` com frontmatter YAML, `_attachments/` para binários e `.openobsidian/srs.json` como único
estado compartilhado. Nada de conta, telemetria ou serviço de terceiro.

Port do **OpenObsidian** para a família craft: o comportamento vem do app Electron (v1.2.1), a
estrutura é do padrão craft. O contrato de vault é o mesmo do OpenObsidianAndroid (files-only), para
que o mesmo cofre abra nos dois.

## O que funciona hoje (v1.1.0)

- **Cofre**: abrir/escolher a pasta (o último cofre volta no próximo start), árvore com notas antes
  das pastas e ordenação por nome ou modificação, filtro instantâneo por nome/tag/alias, **chips de
  tag** com contagem e **backlinks** da nota aberta. A barra lateral é uma só, com três blocos que
  têm altura própria: a árvore, a seção de tags (fechada por padrão, com rolagem própria) e os
  backlinks — nenhum deles empurra nem cobre os outros.
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
  código, imagens de `_attachments/` e links clicáveis; **Edit | Split | Read** num seletor sempre
  visível no cabeçalho da nota (o Split tem divisor arrastável, guardado nas preferências). Um
  `- [ ]` na leitura é uma caixa de verdade: clicar nela alterna a linha no arquivo. `#tag` no corpo
  vira chip e clicar nele filtra a barra lateral. As tabelas usam a largura da página, com faixa de
  cabeçalho, linhas alternadas e quebra por palavra.
- **Importar documento** (`Arquivo → Importar um arquivo Word ou OpenDocument...`): um `.docx` ou
  `.odt` vira uma nota na pasta da nota aberta, com títulos, listas (numeradas inclusive), negrito,
  itálico, links e tabelas. Imagens, gráficos, notas de rodapé e comentários ficam de fora — o mesmo
  corte que o app de origem faz. O `.docx` é lido direto do pacote (ZIP + XML), sem Office instalado.
- **Importar cartões do Anki** (`Arquivo → Importar cartões do Anki (texto)...`): uma exportação de
  texto (`.txt`, `.tsv`, `.csv`) vira notas de `> [!card]`, 100 cartões por nota, com as tags do
  baralho no topo. Mídia de um `.apkg` não entra (ver a tabela abaixo).
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
- **Exportar HTML** (`Arquivo → Exportar a nota como HTML...`): a nota aberta vira uma página
  estática, com o CSS embutido e a paleta seguindo o tema do sistema de quem abre. `[[Nota]]` vira
  `Nota.html` (relativo), callouts viram caixas coloridas por tipo, tabelas ganham cabeçalho e
  alinhamento, e as imagens que a nota mostra são copiadas para um `_attachments/` ao lado da página.
- **Ajuda** (`Ajuda → Atalhos e ajuda`): as teclas, a sintaxe que uma nota aceita e onde cada coisa
  mora no cofre, com link para esta documentação.
- Interface em pt-br, es e ja.

## O que este port não faz (e por quê)

| O que | Por quê |
|---|---|
| **Plugins / sistema de extensões** | O app de origem é Electron: o "plugin" era código JS rodando no renderer. Aqui não há renderer, nem JS, nem carregador dinâmico — é binário Rust. Não é pendência; é divergência estrutural. |
| **Visualizador de PDF/EPUB/DOCX na árvore** | Binários ficam fora da árvore (decisão do port). Um `.docx`/`.odt` entra por **importação**, que vira nota editável — o que um cofre de Markdown quer. PDF/EPUB continuam sendo trabalho do leitor do sistema. |
| **Import de `.apkg` (Anki)** | Ler o pacote exige um motor SQLite. A única opção 100% Rust madura hoje (`turso`/`limbo`) ainda é pré-release e traz uma árvore grande; `rusqlite` compila C, o que o port não aceita. O import de **texto** do Anki (`Arquivo → Importar cartões do Anki (texto)...`) cobre o uso real, e o `.apkg` fica anotado com esse caminho. |
| **Math renderizado** | Continua em unidade própria (precisa de fontes matemáticas + pintor). Hoje o TeX aparece limpo em monoespaçado, sem virar sopa de símbolos. |
| **IA / chat** | Sem IA local, por decisão: nada sai da máquina, e não há provedor embutido. A janela de chat do app de origem não tem equivalente. |
| **Fonte de emoji** | Decisão do João: não se embarca fonte de emoji. As fontes da suíte cobrem alguns pictogramas (que aparecem em monocromático) e não outros; na leitura, um pictograma sem glifo simplesmente **não é desenhado** — um retângulo vazio no meio de um título é pior que um título mais curto. O texto-fonte da nota não muda, e o editor mostra o caractere como está. |

## Roadmap pós-1.0

- **Export PDF**: o app de origem usa o `printToPDF` do Electron; aqui isso é um pipeline de
  impressão próprio (é a mesma peça que os apps de escritório da suíte já têm), então é unidade
  separada, não divergência.
- **Import de `.apkg`** quando houver leitor SQLite puro-Rust maduro (ou um leitor de b-tree próprio).
- **Math renderizado** (fontes + pintor).


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

## QA local (bateria de renderização)

O conteúdo real é o spec: `cargo xtask qa-vault` gera `target/qa-vault/` com 580+ notas, todo tipo de
bloco (títulos, tabela grande, callouts, listas aninhadas, citação com link, destaque, código,
mermaid, query, emoji, frontmatter inválido), 120+ tags e os binários que a árvore mantém fora
(`.txt`, `.docx`, `.pdf`). `cargo xtask qa-shots` abre o app nesse cofre em **1920, 1366, 1024 e
800** px de largura e guarda um PNG de cada em `target/qa-f7/`; `cargo xtask qa-sample` copia ~20
notas do cofre que o usuário tem (somente leitura) para `target/qa-vault-real/`, e as mesmas capturas
saem sobre elas.

A bateria automática é o teste `the_kitchen_sink_lays_out_at_every_window_size` (`cargo test`): nos
quatro tamanhos ele cobra tabela espalhada na largura e sem torre de palavras, nenhum par de rótulos
sobreposto, e (à parte) link clicável e binários fora da árvore. `qa-shots` leva ~12 s e fica fora do
CI; o teste de layout roda sempre.

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
