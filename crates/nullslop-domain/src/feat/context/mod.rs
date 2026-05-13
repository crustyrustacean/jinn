//! Prompt assembly protocol — strategies for building LLM-ready prompts from chat history.
//!
//! This module defines the [`PromptAssembly`] trait and supporting types for
//! assembling conversation context into `LlmMessage` arrays suitable for
//! sending to LLM providers. Each strategy (passthrough, sliding window,
//! token budget, compaction) implements this trait and can be switched
//! at runtime per session.
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
    StrategyFactory, StrategyInfo, StrategySessionData, StrategyState,
};
