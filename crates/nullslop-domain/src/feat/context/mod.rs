//! Context assembly — building LLM-ready prompts from chat history.
//!
//! # Assembly Pipeline
//!
//! Chat history is assembled into LLM-ready messages through these stages:
//!
//! 1. **Pin splitting** — entries are separated into TOP pins, BOTTOM pins,
//!    and working history based on [`PinPosition`](crate::protocol::PinPosition).
//! 2. **Strategy assembly** — the active strategy (passthrough, sliding window,
//!    token budget, compaction) filters/trims the working history within the
//!    session's token budget.
//! 3. **System message construction** — a single [`LlmMessage::System`] is built
//!    by concatenating three sections in priority order (lowest to highest):
//!    - Skills block (`<available_skills>` XML catalog)
//!    - Pinned System entry contents (from TOP-pinned `ChatEntryKind::System`)
//!    - Environment context (date, CWD, persona body, project context files)
//! 4. **Message ordering** — the final array is:
//!    `[System] → [TOP non-System pins] → [strategy output] → [BOTTOM pins] → [last message]`
//!
//! Provider request builders defensively concatenate any remaining `System`
//! messages (e.g., from BOTTOM or RELATIVE pins) into their provider-specific
//! system prompt field, so no system-level context is silently dropped.
//!
//! # Contents
//!
//! This module defines the [`PromptAssembly`] trait and supporting types for
//! assembling conversation context. Each strategy implements this trait and
//! can be switched at runtime per session.
//!
//! Also contains the **ContextActor** (prompt assembly, strategy management,
//! pinning, templates) and **PromptScanActor** (template scanning).

pub mod context_actor;
pub mod env_context;
pub mod prompt_scan_actor;
pub mod prompt_template;
pub mod protocol;
pub mod strategy;

pub use crate::protocol::PromptStrategyId;
pub use strategy::compaction::CompactionStrategy;
pub use strategy::compaction_data::CompactionSessionData;
pub use strategy::discovery::DefaultStrategyDiscovery;
pub use strategy::factory::DefaultStrategyFactory;
pub use strategy::passthrough::PassthroughStrategy;
pub use strategy::sliding_window::SlidingWindowStrategy;
pub use strategy::token_budget::TokenBudgetStrategy;
pub use strategy::token_estimator::{CharRatioEstimator, TokenEstimator, estimate_entry_tokens};
pub use strategy::types::{
    AssembledPrompt, AssemblyContext, PromptAssembly, PromptAssemblyError, StrategyDiscovery,
    StrategyFactory, StrategyInfo, StrategyState,
};
