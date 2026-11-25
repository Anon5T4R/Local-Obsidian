# UPSTREAM-SYNC - Local Notebook

O que neste repo vem de fora, em que revisão, e o que fazer quando o upstream mudar.

## De onde saiu este app

A árvore nasceu de **`Styx-Studios/local-app-template`** (`main` em `55a63ae`) via
`scripts/nascer-app.ps1 -Id localobsidian -Name "Local Obsidian" -Repo Local-Obsidian`, que renomeou
crate/pacote/binário/ids/env vars/pastas e gerou GUIDs novos do MSI (o app se chamava então Local Obsidian e `localobsidian`; o produto virou Local Notebook, `localnotebook`, na 1.10.0). O ícone nasceu no próprio repo
(ver `assets/app-icon/README.md`).

O **comportamento** portado vem do **OpenObsidian** (`C:\Dev\OpenObsidian\desktop`, Electron + React
+ TS, v1.2.1): o TypeScript é spec, não estrutura. O que já foi portado e as divergências
conscientes ficam registrados aqui, seção a seção, conforme o port avança.

## Divergências conscientes (port vs. Electron)

| O que | Electron | Aqui | Por quê |
|---|---|---|---|
| Frontmatter na leitura | `remark-frontmatter` tira o bloco | `preview::prepare` descarta o bloco antes de parsear | mesmo resultado sem HTML no meio |
| Backlinks com âncora | só `backlinks[nomeDaNota]` (um `[[Nota#Seção]]` não aparece) | conta também (`parse_target` compara a nota) | menção é menção; era um buraco do upstream, não uma decisão |
| Âncoras de heading | `slugify` ASCII (`\w`), ids persistidos no HTML | slug Unicode (mantém acentos), id só para casar `[[Nota#Seção]]` | id não vai a arquivo; manter a letra evita `seção`→`seco` |
| Callout com formatação no título | HTML com ênfase | título em texto puro, corpo preserva ênfase | o modelo de blocos não tem spans no título |
| Escrita | autosave 800 ms + `safe-write` | igual (temp + rename, retry no Windows) | — |
| Busca | `toLowerCase` + `includes` (limitação #1 Keep) | igual, **sem** folding de acento e **sem** stemming | decisão do upstream mantida |
| Regex na busca | `new RegExp` do JS | crate `regex` (mesmo `.*`), sintaxe do Rust | JS tem backreference/lookahead; Rust não (diferença anotada) |
| Embed | injeta `<div class="embed">` no HTML | vira callout `> [!embed] Nota › Seção` | sem pipeline HTML |
| Watcher de arquivos | chokidar (`awaitWriteFinish`, eventos por tipo) | `notify` 8, recursivo, com debounce de 300 ms; `.saving`/dotfiles/`.openobsidian` descartados; **a nota aberta vence** (buffer sujo não é sobrescrito, avisa na status bar) | o do upstream não filtra as escritas do próprio app nem o `srs.json`; aqui o "próprio write" é reconhecido por path + instante, não por tempo de settle |
| Math | KaTeX | TeX em monoespaçado (pendente) | ver nota abaixo |
| Binários (`.pdf`/`.docx`/`.odt`/`.epub`/`.txt`) na árvore | aparecem | aparecem apagados entre as notas e as pastas e abrem no app do sistema; não são notas — busca, wikilinks e backlinks não os veem | um documento do cofre não pode sumir da árvore só por não ser editável |
| SRS/flashcards | `srs.ts` + `cards.ts` + main/IPC (`srs.json`) | crate `localnotebook-review` (SM-2 + extração + arquivo) + painel Review | mesmo algoritmo e **mesmo arquivo**; o app só lê/escreve `.openobsidian/srs.json` |
| Estado do SRS em memória | cache por vault no processo main | `VaultState.srs`, re-sincronizado no reload e depois de cada save | o Electron precisa do cache porque o renderer é outro processo; aqui é um `BTreeMap` no mesmo dono do buffer |
| Import `.apkg` | `main/apkg.ts` (zip + SQLite) | **adiado** (F5, com o João) | ler `.apkg` pede SQLite (dep com C) ou um parser de banco próprio; fora da regra pure-Rust |
| Grafos | `GraphView.tsx` (força dirigida, d3, vault inteiro) | **removido por decisão do João**: o grafo global virou um **mapa de conexões** expansível — a nota no centro, as conexões diretas em anel, o `+` de cada nota abrindo um nível por vez, clique abre a nota | estética não pagava o preço: o do Electron é pesado, uma bola de 500 nós não se lê e o nosso também não. O mapa é determinístico, leve e navegável, que é a utilidade real |
| Calendário | `CalendarPopover` + `utils/calendar.ts` | popover próprio: grade de 6 semanas, dia com nota marcado, cria a diária a partir de `_templates/daily.md` | mesma conta de mês (clamp de dia) e de nota diária `YYYY-MM-DD` |
| Templates | `TemplateModal` + 6 embutidos + `exampleTemplate` | 4 embutidos (Blank/Daily/Meeting/Project) + notas do usuário em `_templates/`; variáveis `{{title}}`, `{{date[:fmt]}}`, `{{time}}`, `{{datetime}}` | o `exampleTemplate` é demo de bloco de query; os outros dois embutidos não tinham uso no port |
| Nota diária | nome `YYYY-MM-DD` na raiz + template `daily` | igual | — |
| ODT/DOCX | `odtToHtml` + mammoth → HTML → turndown (`.md` ao lado do arquivo) | crate `localnotebook-convert`: ZIP + XML → **Markdown direto**; a nota vai para a pasta da nota aberta | sem pipeline HTML no port, e a conversão em dois passos perdia tabela e aninhamento de lista |
| Export ODT/DOCX | **não existe** no upstream (só export HTML/PDF) | também não | não se inventa direção que o spec não tem |
| Conversão de documento | botão na barra, com o arquivo já na árvore | item de menu + seletor nativo, porque a árvore agora lista os binários e um botão ali só serviria à nota aberta | sem pipeline HTML no port, e a conversão em dois passos perdia tabela e aninhamento de lista |
| Checkbox na leitura | clicável (CodeMirror resolve a posição) | clicável: a leitura conta qual caixa é (`n`-ésima) e `toggle_task` acha a mesma linha no fonte | `prepare` reescreve o texto, então deslocamento de byte não sobrevive; a contagem sim |
| Import Anki `.apkg` | `sql.js` (SQLite em WASM) + `fzstd` para a mídia | **não** — só a exportação de texto | ver a nota abaixo |
| Import Anki texto | `srs:anki-pick` → `srs:anki-write`, 100 cartões por nota | igual (mesmo `chunk_cards`, mesma colisão de nome com sufixo numérico); notas soltas, não uma pasta por baralho | o upstream cria uma pasta por baralho quando a exportação passa de 100; aqui as notas ficam na raiz com sufixo |
| Ajuda | `HelpModal` com 5 abas | janela única: teclas, sintaxe da nota, onde cada coisa mora, link do README | 5 abas para uma janela que se abre raramente é peso, não ajuda |
| Export HTML | `remark` → HTML com CSS fixo claro, nota ativa | `localnotebook_md::html`: o mesmo modelo de blocos da leitura, CSS embutido com paleta clara/escura por `prefers-color-scheme`, callouts coloridos por tipo, tabela com cabeçalho e alinhamento | exportar o que se vê; o CSS do upstream não conhece callout e obriga o leitor a usar tema claro |
| Wikilink no HTML | fica texto literal (`[[Nota]]`) | vira `Nota.html` (+ `#seção` do título) | exportar as notas para a mesma pasta faz o link funcionar; literal não leva a lugar nenhum |
| Imagens no HTML | aponta para o caminho do vault | copiadas para `_attachments/` ao lado da página | o arquivo sai do cofre; sem a cópia a página abre com imagens quebradas |
| Export PDF | `printToPDF` do Electron | `localnotebook-pdf`: os blocos da leitura em A4 (56 pt de margem), paginação própria, glifos com fonte embutida e mapa Unicode (texto selecionável), callout/tabela/código desenhados | não é divergência de produto: o Electron imprime HTML, aqui não há HTML — e a fase entrega texto de verdade, não imagem da página |
| Barra lateral | explorer + "TAGS 150" fechada + "BACKLINKS 1" no rodapé | uma barra, três blocos com altura própria: árvore (rola), tags (fechada por padrão, rolagem própria) e backlinks (rolagem própria) | impedir que 148 tags ou 137 backlinks cubram a árvore: o conteúdo de cada bloco é limitado ao espaço dele |
| Tabelas na leitura | tabela HTML do remark | renderer próprio: colunas medidas pelo conteúdo e distribuídas na largura, faixa de cabeçalho, linhas alternadas, grade e quebra por palavra | `egui::Grid` mede a célula como quase nada dentro de um `with_layout` e cai no mínimo de coluna: a tabela virava torre de palavras partidas |
| Modo de leitura | abas Editor/Dividir/Visualizar | seletor segmentado com as três opções sempre visíveis; o divisor do Split é arrastável e persistido | um botão que cicla esconde o estado: parecia que a leitura não renderizava |
| Espaçamento inline | CSS | espaçamento zero entre spans da mesma linha | o espaçamento padrão do egui abria um vão visível em volta de cada link |
| Comandos de barra | `insertables.ts` (lista única do menu Inserir e do `/`), gatilho no começo da palavra, `\w*`, snippet + cursor | catálogo único em `slash.rs` (id, aliases, trecho, placeholder), mesmo gatilho (começo da palavra), **`/toc` gerado dos títulos**, fora de bloco de código, `Esc` fecha de verdade, e o exemplo entra **selecionado** (digitar substitui); bloco tomado no meio da linha ganha a linha própria | o menu Inserir não foi portado (o `/` é a porta); `toc` é do port; `\w` aceitaria `_`, aqui só letra e dígito; o upstream não fecha bloco de código nem mantém o popup fechado depois do `Esc` |
| Tarefa em lista (`/task`) | insere três itens, o cursor no segundo | um item só | três caixas vazias de uma vez é ruído; o Enter já continua a lista |
| Tags no corpo (leitura) | texto | `#tag` vira chip na cor de acento; clicar filtra a barra lateral | é a mesma tag que o índice conhece (mesmas regras de caractere) |
| Emoji | fonte colorida do sistema | sem fonte de emoji embarcada; pictograma sem glifo não é desenhado na leitura | decisão do João; ver o README |
| Tema | modal de Configurações com dois botões (Claro/Escuro), padrão escuro | `Visualizar → Tema` com **três** opções — Sistema (padrão), Claro, Escuro — persistido em `prefs.json`; acento por tema e callout escurecido no claro | num app nativo seguir o desktop é o esperado (o Electron nunca teve "sistema"); o claro precisava da paleta revisada, não só de uma flag |
### Math: avaliação do RaTeX (pendente, unidade própria)

O `ratex` do local-sheets (`crates/math`, 489 linhas) roda LaTeX → `MathList` (glifos, réguas,
caminhos) com teto de entrada e `catch_unwind`, e plugaria como crate. **Não entrou** porque o
pintor precisa de *fontes matemáticas* (o `MathList` traz `Face::MainRegular/Bold/Fraktur…`, não os
contornos) - são MBs de assets com licença própria para registrar em `ATTRIBUTION`, mais um pintor
egui. Fica como unidade separada (com o João), junto dos pins `ratex-* =0.1.14` (4 crates).

### `.apkg`: por que não entrou (pesquisa de 2026-11-15)

O upstream lê `.apkg` com **`sql.js`** (SQLite compilado para WASM) dentro do Electron. Do lado Rust, as
opções foram levantadas por evidência:

| Caminho | Estado | Veredito |
|---|---|---|
| `rusqlite` (feature `bundled`) | 155 M downloads, maduro | **não**: compila SQLite em C, e o port é pure Rust (sem C no grafo, regra do `AGENTS.md`) |
| `turso` / `turso_core` (`limbo`) | reescrita em Rust do SQLite, 1,6 M downloads, mas `0.9.0-pre.5` (12/11/2026) | **não agora**: pré-release, API instável, árvore grande — para ler 3 tabelas de um baralho é um martelo de demolição |
| `sqlite-parser` | 26 k downloads totais, 104 recentes, último release 08/2025 | **não**: praticamente morto, e é parser de *SQL*, não leitor de b-tree |
| leitor de b-tree próprio (read-only) | não existe no ecossistema; o formato é documentado | **caminho futuro**: ~400 linhas, sem dep, com o `apkg.test.ts` do upstream como spec |

Enquanto isso, a **exportação de texto** cobre o uso real (o Anki exporta `.txt`/`.tsv` em dois
cliques) e o import dela está implementado. O `.apkg` fica como limitação no README, com este caminho.

### Pendências herdadas do escopo

- Autocomplete do `[[`: nome e alias por `contains` (sem fuzzy de distância nem ranking por uso).
- O painel de busca não tem histórico de queries (o upstream tampouco).
- Sem sistema de plugins: o do upstream era JS no renderer, e aqui não há renderer — divergência
  estrutural, documentada no README.
- Import de `.apkg` adiado (precisa de SQLite; ver a nota acima).
- Math/TeX renderizado segue em unidade própria (fontes matemáticas + pintor); hoje é monoespaçado.

### Decisão do arquiteto 2026-11-15: limitações #4 e #5 do upstream

As duas limitações de `docs/limitations.md` do upstream que tocam a gramática de query ficam
**idênticas ao upstream**, de propósito:

- **#4 — vírgula = AND** (e não OR, como a documentação de busca sugere).
- **#5 — `limit:` conta como "houve intenção"** de busca, mesmo quando não limita nada.

Motivo: **a gramática de query é uma só** entre o desktop e o `OpenObsidianAndroid` (regra do próprio
`limitations.md`), e o Android está congelado. Mudar o significado de qualquer das duas aqui faria o
mesmo cofre responder diferente nos dois lados. Quando o Android for tocado de novo, #4 e #5 caem
**juntas**, com release notes nos dois repos.

## Spec funcional (somente leitura)

| O que | Onde | Regra |
|---|---|---|
| Comportamento (tree sort, filtro da sidebar, frontmatter, wikilinks/embeds, safe-write, link-rewrite) | `C:\Dev\OpenObsidian\desktop\src` | portar comportamento; nunca copiar estrutura React/Electron |
| Decisões travadas (11 itens com veredito) | `docs/limitations.md` do upstream | itens "Keep" são requisito: não "consertar" no port |
| Gramática de query | `utils/noteQuery.ts` + `NoteQuery` do Android | **a mesma gramática** em `crates/vault/src/query.rs`: `tag`/`tags`, `path`/`pasta`, `has`/`tem`, campo de frontmatter livre, `sort`/`ordenar` (titulo/modificado/criado/caminho + `desc`) e `limit`/`limite`; linha inválida volta em `mistakes` | uma só gramática, igual ao Android de propósito. `criado` cai em `modificado` (o cofre não guarda data de criação) |
| Diagnóstico do cofre | **não existe** no upstream | janela própria: links quebrados (com quem os escreveu), notas órfãs e nomes duplicados, cada linha abre a nota | pedido do ledger do vault do João (item P1 não coberto pelo Electron) |
| Flashcards de IA | **não existe** no upstream (o Electron não tem IA) | `IA → Create flashcards from this note`: o modelo escreve `[!card]` e o resultado vira **nota nova ao lado** (`<nota> AI`), original intacta, undo = apagar a criada | padrão Ação-Artefato da suíte (`padrao-craft.md` §Superfície de IA), destrutividade zero por decisão do João |
| Contrato de vault | backlog B18 (dev-notes) | files-only: `.md` + frontmatter + wikilinks + embeds, `_attachments/`, `.openobsidian/srs.json` como único estado compartilhado |

## Forks vendored (`libs/`)

| Árvore | Origem | Revisão | Sync |
|---|---|---|---|
| `libs/egui` | storytold/egui (fork do egui) | `6d518fc799681571963842f95b55a6b68431b15a` | skill `sync-upstream`; patches da suíte (bidi, scripting, viewport hooks) |
| `libs/winit` | storytold/winit (fork do winit) | `a517b3846ee8457d84ee4975110bfda6797c44dc` | idem |
| `libs/craft-libs` | storytold/craft-libs | base `1e6f1278` | idem, **exceto** `crates/update` (abaixo) |
| `libs/craft-libs/crates/update` | **`Styx-Studios/local-libs`** | `HASHES.update` da fonte | **fora do sync-upstream**: fork permanente consciente. Mudança vai na fonte e replica com `scripts/sync.ps1 -Mode push`; a cópia aqui é espelho e nunca se edita in place |
| `libs/winprint.rs` | alampy/winprint.rs (BSD-3) | `417ac3c` (+ `winprint-gdi` no rev antigo) | usado por `craft-libs/crates/print`; upstream vivo, patches genéricos vão por PR |
| `libs/rustls-rustcrypto` | fork da suíte | - | o `craft-update` pina este fork (o upstream alpha puxa rustls-webpki 0.102 só por `alg_id`) |
| `assets/craft-fonts` | storytold/craft-fonts | `b076417b5880bd4154bd22f00695b1d28c6bea83` | build input (`CRAFT_FONTS_DIR`); atualiza trocando a árvore inteira e o rev em `packaging/fonts.sh`, `packaging/windows/fonts.ps1` e no CI |

Regra do repo: nada é lido de `C:\Dev\storytold` em build, teste ou release. As menções a
`storytold/*` em código, docs e atribuições são referência histórica e obrigação de licença.

## Catálogo da suíte

`craft_update::suite` (na crate espelhada) é a fonte única dos `App` de cada app. A entrada
`LOCALNOTEBOOK` foi adicionada em `Styx-Studios/local-libs` (`crates/update/src/suite.rs`) e replicada
para os apps com `scripts/sync.ps1 -Mode push`.
