//! Context assembly - building LLM-ready prompts from chat history.
//!
//! # Assembly Pipeline
//!
//! Chat history is assembled into LLM-ready messages through these stages:
//!
//! 1. **Pin splitting** - entries are separated into TOP pins, BOTTOM pins,
//!    and working history based on [`PinPosition`](crate::protocol::PinPosition).
//! 2. **Compaction** - if the working history exceeds the session's token budget,
//!    entries are trimmed newest-to-oldest (preserving pinned entries) and a
//!    compaction system prompt is injected.
//! 3. **System message construction** - a single [`LlmMessage::System`] is built
//!    by concatenating sections in priority order (lowest to highest):
//!    - Skills block (`<available_skills>` XML catalog)
//!    - Pinned System entry contents (from TOP-pinned `ChatEntryKind::System`)
//!    - Environment context (date, CWD, persona body, project context files)
//!    - Tool context block
//!    - Compaction prompt (when trimming occurred)
//! 4. **Message ordering** - the final array is:
//!    `[System] → [TOP non-System pins] → [compacted working history] → [BOTTOM pins] → [last message]`
//!
//! Provider request builders defensively concatenate any remaining `System`
//! messages (e.g., from BOTTOM or RELATIVE pins) into their provider-specific
//! system prompt field, so no system-level context is silently dropped.
//!
//! # Contents
//!
//! This module defines the compaction assembly logic and supporting types.
//! Also contains the **ContextActor** (prompt assembly, pinning, templates)
//! and **PromptScanActor** (template scanning).

pub mod assemble;
pub mod assembly_state;
pub mod context_size_actor;
pub mod context_files_scan_actor;
pub mod env_context;

pub mod prompt_scan_actor;
pub mod prompt_template;
pub mod protocol;
pub mod strategy;
pub mod tool_prompt;

pub use strategy::token_estimator::{
    CharRatioEstimator, TokenEstimator, estimate_entry_tokens, estimate_tool_schema_tokens,
};
