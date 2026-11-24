//! Local Notebook's note format, with no UI and no filesystem: YAML frontmatter, `#tags`,
//! `[[wikilinks]]`, `![[embeds]]`, link rewriting on rename, and the preview model.
//!
//! The rules are ported from OpenObsidian's TypeScript (`frontmatter.ts`, the tag extraction in
//! `vaultStore.ts`, `linkResolver.ts`/`link-rewrite.ts`, `embeds.ts` and `markdownTransforms.ts`),
//! and its tests are the tests here: the port is judged by the behaviour those tests pin, not by
//! the shape of the original code.

pub mod code;
pub mod embeds;
pub mod frontmatter;
pub mod html;
pub mod links;
pub mod outline;
pub mod preview;
pub mod rewrite;
pub mod tags;

pub use frontmatter::Frontmatter;
pub use links::{Target, display_target, parse_target};
pub use preview::{Block, Span, Task};
