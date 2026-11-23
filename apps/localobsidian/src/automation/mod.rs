//! Agent-native Local Obsidian: the commands the vault app offers an agent, a headless session that
//! runs them, the MCP server on top of it and the control channel of the running window.
//!
//! The plumbing is the suite's `craft-mcp` (`libs/craft-libs/crates/mcp`, source `local-libs`):
//! the stdio server, the `run --script` steps with `"$N"` references, the
//! `--automation-read-root`/`--automation-write-root` flags and `--connect PORT`. What this module
//! adds is Local Obsidian's own: the command catalogue ([`catalogue`]), the session ([`session`])
//! over the [`crate::vault_state::VaultState`] the app already has, the MCP handler ([`mcp`]) and
//! the window's loopback control channel ([`control`]).
//!
//! A vault is the document: the headless session scans it, reads and writes notes, renames them
//! (rewriting their links), searches it and exports from it exactly through the app's own code.
//! What needs the window (pickers, the viewport, the review session, the AI chat, the updater) says
//! `appOnly`.

pub mod catalogue;
pub mod cli;
pub mod control;
pub mod launch;
pub mod mcp;
pub mod session;
