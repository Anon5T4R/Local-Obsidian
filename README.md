# Local Obsidian

Notas, wikilinks e anexos numa pasta comum — app desktop da suíte Local, em **Rust puro + egui
forkado** (sem webview, sem React, sem servidor). Um vault é a pasta que o usuário já tem:
`.md` com frontmatter YAML, `_attachments/` para binários e `.openobsidian/srs.json` como único
estado compartilhado. Nada de conta, telemetria ou serviço de terceiro.

Port do **OpenObsidian** para a família craft: o comportamento vem do app Electron (v1.2.1), a
estrutura é do padrão craft. O contrato de vault é o mesmo do OpenObsidianAndroid (files-only), para
que o mesmo cofre abra nos dois.

## O que funciona hoje

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
- **Comandos de barra**: `/` no começo de uma palavra (início da linha ou depois de espaço, fora de
  bloco de código) abre a lista do que se pode inserir — títulos `h1`/`h2`/`h3`, `bold`, `italic`,
  `strike`, `highlight`, `inline`, `comment`, `list`, `numlist`, `task`, `table`, `code`, `quote`,
  `hr`, `wikilink`, `embed`, `link`, `image`, `card`, `cloze`, `toc` (sumário montado dos títulos da
  nota), `callout`/`tip`/`warning`/`danger`, `frontmatter`, `date` e `time`. `↑`/`↓` andam, `Enter`/`Tab`
  aceitam, `Esc` dispensa; o trecho de exemplo vem **selecionado**, então digitar substitui, e um
  bloco tomado no meio da linha ganha a linha própria antes.
- **Leitura**: títulos, listas, citação, callouts (`> [!warning]`), tabelas alinhadas, blocos de
  código, imagens de `_attachments/` e links clicáveis; **Edit | Split | Read** num seletor sempre
  visível no cabeçalho da nota (o Split tem divisor arrastável, guardado nas preferências). Um
  `- [ ]` na leitura é uma caixa de verdade: clicar nela alterna a linha no arquivo. `#tag` no corpo
  vira chip e clicar nele filtra a barra lateral. As tabelas usam a largura da página, com faixa de
  cabeçalho, linhas alternadas e quebra por palavra.
- **Leitura com medida de livro**: a coluna tem 820 px e fica centrada, com degraus de tipo de
  verdade (H1 29, H2 21, corpo 15) e ritmo entre blocos — o texto não vira mais uma parede de 150
  caracteres por linha. O frontmatter aparece como **card de propriedades** no topo (tipo, aliases e
  tags em chips, na ordem do arquivo), o código inline é um chip neutro, a tabela tem o respiro de
  ~40 px por linha, e o painel de código do Split ganha realce (fences, títulos, citações, `#tags`,
  `[[links]]`, `==destaques==`). **Zoom da renderização** de 0.8 a 1.6 (`Ctrl+=`, `Ctrl+-`,
  `Ctrl+0` ou `Visualizar → Zoom`), guardado nas preferências.
- **Documentos da pasta**: `.pdf`, `.docx`, `.odt`, `.epub` e `.txt` que estão no cofre aparecem na
  árvore — apagados, entre as notas e as pastas — e **abrem no app do sistema** com um clique; o
  menu de contexto de uma pasta (ou de um documento) mostra no gerenciador de arquivos. Continuam
  não sendo notas: busca, wikilinks e backlinks não os veem.
- **IA** (`IA → Conversar sobre a nota`): painel de chat que manda a nota aberta como contexto e
  recebe a resposta em pedaços (SSE), com `Enviar`, `Parar`, `Inserir na nota` e `Limpar`. A conexão
  (`IA → Conexão…`) aceita qualquer provedor compatível com a API da OpenAI — Ollama e llama-server
  são uma URL base e um modelo —, busca a lista de modelos do provedor e guarda a chave **no
  keychain do sistema**, nunca num arquivo. Sem conexão, o painel diz o que falta. É o `local-ai`
  compartilhado da suíte, com nenhum servidor nosso no caminho: nada sai da máquina sem o endpoint
  que o usuário configurou.
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
  fecha). Baralhos: cofre inteiro, a nota aberta ou uma tag, **escolhidos no próprio painel** (o
  seletor lista só o que tem cards, com a contagem de cada um) e o cabeçalho diz o escopo e os
  números dele (cards no baralho · vencidos hoje · feitos na sessão). Cards renderizam o Markdown da
  nota; o painel de estatísticas acompanha o baralho da sessão, com retenção, facilidade média,
  previsão de 14 dias e a fila dos próximos cards com a nota de origem. O cronograma mora em
  `.openobsidian/srs.json`, **o mesmo arquivo que o OpenObsidianAndroid lê** (mesmos campos, mesma
  ordem, nada de estado extra); o cartão guarda o id estável (hash da pergunta), então editar a
  resposta não zera o agendamento.
- **Query blocks**: uma cerca ` ```query ` não é código para ler, é pergunta para responder —
  `tag:`, `path:`, `has:` e qualquer campo do frontmatter (`tipo: patologia`), com `sort: titulo|
  modificado|caminho [desc]` e `limit:`. A leitura mostra "N resultados" e a lista clicável (as notas
  que casam, na ordem pedida), com `+N more` para revelar o resto; a linha que a gramática não entende
  aparece como aviso em vez de ser ignorada. Mesma gramática do app Android.
- **Flashcards de IA** (`IA → Create flashcards from this note`, `Ctrl+Shift+K` ou o menu da nota na
  árvore): o modelo escreve `> [!card] pergunta` + `> ==resposta==` a partir da nota aberta e o
  resultado vai para **uma nota nova ao lado** (`<nota> AI.md`), com o frontmatter e as tags herdados
  mais `flashcards-ai`; a nota original não é tocada e o undo apaga só a criada. O diálogo escolhe
  cards por seção (1/2/3/5) e idioma, mostra o stream com Stop e reabre com a razão se a engine falhar.
- **Diagnóstico do cofre** (`Visualizar → Diagnóstico do cofre`): links que apontam para nota
  inexistente (com quem os escreveu), notas órfãs (sem link de entrada nem de saída) e nomes
  duplicados em pastas diferentes — cada linha abre a nota.
- **Conexões** (`Visualizar → Mostrar as conexões`): a nota aberta no centro e as conexões diretas
  dela (links que saem e links que entram) em anéis por salto, com o `+` de cada nota abrindo as
  conexões *dela* — o mapa cresce só pelo pedaço do cofre que interessa. Clicar numa nota abre a
  nota; o menu da direita abre, recolhe ou começa o mapa por ali; a barra tem "Recolher tudo" e
  "Começar pela nota aberta". Layout determinístico (sem física): o mesmo cofre desenha o mesmo mapa.
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
- **Exportar PDF** (`Arquivo → Exportar a nota em PDF...`): a nota vira **páginas A4 com texto de
  verdade** — os glifos vão com as fontes embutidas (subset) e o mapa Unicode, então selecionar,
  buscar e copiar funcionam no leitor de PDF. Sai o que a leitura desenha: títulos, parágrafos com
  negrito/itálico/riscado/código/destaque, listas e tarefas com as sub-listas indentadas por nível,
  citação, callout com a cor do tipo, tabela com grade e colunas medidas, código em mono, as
  imagens do cofre embutidas (até 16 MB e 64 MP — o tamanho sai do cabeçalho do arquivo, então um
  PNG gigante vira o `[image: …]` sem nunca ser decodificado; sem o arquivo, ou num formato que o
  escritor não lê, fica o mesmo marcador) e o número da página. **Links** para a web viram áreas clicáveis no leitor; um
  `[[wikilink]]` fica texto, porque só existe dentro do app.
- **Ajuda** (`Ajuda → Atalhos e ajuda`): as teclas, a sintaxe que uma nota aceita e onde cada coisa
  mora no cofre, com link para esta documentação.
- Interface em pt-br, es e ja.
- **Tema** (`Visualizar → Tema`): **Sistema** (segue o claro/escuro do desktop, o padrão), **Claro** ou
  **Escuro**, guardado nas preferências. Só o acento é nosso (o violeta do ícone, escurecido no branco
  e clareado no escuro, para link, tag e citação); o resto do cinza é o do tema — e os callouts, que
  têm cor própria, escurecem no claro para o título continuar legível.

## O que este port não faz (e por quê)

| O que | Por quê |
|---|---|
| **Plugins / sistema de extensões** | O app de origem é Electron: o "plugin" era código JS rodando no renderer. Aqui não há renderer, nem JS, nem carregador dinâmico — é binário Rust. Não é pendência; é divergência estrutural. |
| **Visualizador de PDF/EPUB/DOCX dentro do app** | Os documentos aparecem na árvore e abrem no leitor do sistema — um PDF não é uma nota e o app não tenta renderizá-lo. Um `.docx`/`.odt` que precisa virar texto entra por **importação**, que gera nota editável. |
| **Import de `.apkg` (Anki)** | Ler o pacote exige um motor SQLite. A única opção 100% Rust madura hoje (`turso`/`limbo`) ainda é pré-release e traz uma árvore grande; `rusqlite` compila C, o que o port não aceita. O import de **texto** do Anki (`Arquivo → Importar cartões do Anki (texto)...`) cobre o uso real, e o `.apkg` fica anotado com esse caminho. |
| **Math renderizado** | Continua em unidade própria (precisa de fontes matemáticas + pintor). Hoje o TeX aparece limpo em monoespaçado, sem virar sopa de símbolos. |
| **Modelo de IA embutido** | O app é cliente: fala com o endpoint compatível com a API da OpenAI que o usuário configurar (Ollama, llama-server, provedor remoto). Não embarca pesos nem roda inferência — isso é trabalho do `local-ai`, que é só o cliente compartilhado da suíte. |
| **Fonte de emoji** | Decisão do João: não se embarca fonte de emoji. **Emoji nunca é desenhado** na leitura (um pictograma monocromático no meio do texto lê como erro), e as faixas de símbolo — setas, operadores matemáticos — aparecem quando as fontes da suíte têm o glifo. O texto-fonte da nota não muda e o editor mostra o caractere como está. |

## Roadmap pós-1.0

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
| xtask: `ci`, `fmt`, `assets`, `layers`, `version`, `ico`, `qa-vault`/`qa-shots`/`qa-sample` | `xtask/` |

## Estrutura

```
apps/localobsidian/  app: main.rs (args, ícone, janela), app.rs (menus/About/página), i18n.rs,
                     pickers.rs, fonts.rs, suite_entry.rs, build.rs (ícone, i18n, fontes)
crates/              camadas `localobsidian-*` (registradas em xtask/src/layers.rs TABLE)
assets/              app-icon/, craft-fonts/ (vendored), ATTRIBUTION.md
i18n/locales/        catálogos de tradução (context ⇥ fonte EN ⇥ tradução)
libs/                forks vendored: craft-libs, egui, winit, rustls-rustcrypto, winprint.rs
packaging/           env.sh, fonts.sh, icons.sh, minisign.pub, linux/, arch/, windows/
xtask/               ci, fmt, assets, layers, version, ico, qa-vault/qa-shots/qa-sample
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
cargo xtask ci                        # o gate local: o mesmo que o CI (fmt, clippy, build, assets,
                                      # layers, testes, cargo-deny, espelhos do local-libs)
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
