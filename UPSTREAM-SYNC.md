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

## Divergências conscientes (port vs. Electron)

| O que | Electron | Aqui | Por quê |
|---|---|---|---|
| Frontmatter na leitura | `remark-frontmatter` tira o bloco | `preview::prepare` descarta o bloco antes de parsear | mesmo resultado sem HTML no meio |
| Backlinks com âncora | só `backlinks[nomeDaNota]` (um `[[Nota#Seção]]` não aparece) | conta também (`parse_target` compara a nota) | menção é menção; era um buraco do upstream, não uma decisão |
| Âncoras de heading | `slugify` ASCII (`\w`), ids persistidos no HTML | slug Unicode (mantém acentos), id só para casar `[[Nota#Seção]]` | id não vai a arquivo; manter a letra evita `seção`→`seco` |
| Callout com formatação no título | HTML com ênfase | título em texto puro, corpo preserva ênfase | o modelo de blocos não tem spans no título |
| Checkbox na leitura | clique alterna a caixa | `Ctrl+Enter` no editor | o índice do checkbox por fonte não mapeia 1:1 com a leitura (embeds/listas aninhadas) |
| Escrita | autosave 800 ms + `safe-write` | igual (temp + rename, retry no Windows) | — |
| Busca | `toLowerCase` + `includes` (limitação #1 Keep) | igual, **sem** folding de acento e **sem** stemming | decisão do upstream mantida |
| Regex na busca | `new RegExp` do JS | crate `regex` (mesmo `.*`), sintaxe do Rust | JS tem backreference/lookahead; Rust não (diferença anotada) |
| Embed | injeta `<div class="embed">` no HTML | vira callout `> [!embed] Nota › Seção` | sem pipeline HTML |
| Watcher de arquivos | chokidar (`awaitWriteFinish`, eventos por tipo) | `notify` 8, recursivo, com debounce de 300 ms; `.saving`/dotfiles/`.openobsidian` descartados; **a nota aberta vence** (buffer sujo não é sobrescrito, avisa na status bar) | o do upstream não filtra as escritas do próprio app nem o `srs.json`; aqui o "próprio write" é reconhecido por path + instante, não por tempo de settle |
| Math | KaTeX | TeX em monoespaçado (pendente) | ver nota abaixo |
| Binários (PDF/DOCX/EPUB/ODT) na árvore | aparecem | fora da árvore | listar o que não abre é pior que não listar |
| SRS/flashcards | `srs.ts` + `cards.ts` + main/IPC (`srs.json`) | crate `localobsidian-review` (SM-2 + extração + arquivo) + painel Review | mesmo algoritmo e **mesmo arquivo**; o app só lê/escreve `.openobsidian/srs.json` |
| Estado do SRS em memória | cache por vault no processo main | `VaultState.srs`, re-sincronizado no reload e depois de cada save | o Electron precisa do cache porque o renderer é outro processo; aqui é um `BTreeMap` no mesmo dono do buffer |
| Import `.apkg` | `main/apkg.ts` (zip + SQLite) | **adiado** (F5, com o João) | ler `.apkg` pede SQLite (dep com C) ou um parser de banco próprio; fora da regra pure-Rust |
| Grafos | `GraphView.tsx` (força dirigida, d3) | anéis determinísticos (hub no centro, `MAX_NODES` 240), rótulos e clique abre a nota | força que nunca assenta é bonita no print e ruim de ler; o mesmo vault desenha igual |
| Calendário | `CalendarPopover` + `utils/calendar.ts` | popover próprio: grade de 6 semanas, dia com nota marcado, cria a diária a partir de `_templates/daily.md` | mesma conta de mês (clamp de dia) e de nota diária `YYYY-MM-DD` |
| Templates | `TemplateModal` + 6 embutidos + `exampleTemplate` | 4 embutidos (Blank/Daily/Meeting/Project) + notas do usuário em `_templates/`; variáveis `{{title}}`, `{{date[:fmt]}}`, `{{time}}`, `{{datetime}}` | o `exampleTemplate` é demo de bloco de query; os outros dois embutidos não tinham uso no port |
| Nota diária | nome `YYYY-MM-DD` na raiz + template `daily` | igual | — |

### Math: avaliação do RaTeX (pendente, unidade própria)

O `ratex` do local-sheets (`crates/math`, 489 linhas) roda LaTeX → `MathList` (glifos, réguas,
caminhos) com teto de entrada e `catch_unwind`, e plugaria como crate. **Não entrou** porque o
pintor precisa de *fontes matemáticas* (o `MathList` traz `Face::MainRegular/Bold/Fraktur…`, não os
contornos) - são MBs de assets com licença própria para registrar em `ATTRIBUTION`, mais um pintor
egui. Fica como unidade separada (com o João), junto dos pins `ratex-* =0.1.14` (4 crates).

### Pendências herdadas do escopo

- Autocomplete do `[[` existe e é testado; falta só o fuzzy por alias (hoje é `contains`).
- O painel de busca não tem histórico de queries (o upstream tampouco).
- Sem IA (o port é files-only, por contrato).
- Import de `.apkg` adiado para o F5 (precisa de SQLite; ver a tabela acima).
- Checkbox clicável na leitura segue pendente (hoje é `Ctrl+Enter` no editor).

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
