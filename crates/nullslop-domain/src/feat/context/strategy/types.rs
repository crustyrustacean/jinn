//! Core types for prompt assembly.

use async_trait::async_trait;
use error_stack::Report;
use serde::{Deserialize, Serialize};
use wherror::Error;

use crate::feat::context::strategy::compaction_data::CompactionSessionData;
use crate::protocol::{ChatEntry, LlmMessage, PromptStrategyId, SessionId, ToolDefinition};

/// Typed strategy state — each variant carries its strategy-specific persistent data.
///
/// Stored directly on `ChatSessionState` and serialized with the session.
/// Replaces the opaque `serde_json::Value` blob pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StrategyState {
    Passthrough,
    SlidingWindow,
    TokenBudget,
    Compaction(CompactionSessionData),
}

/// Error type for prompt assembly operations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct PromptAssemblyError;

/// The result of assembling a prompt for an LLM.
#[derive(Debug, Clone)]
pub struct AssembledPrompt {
    /// System prompt, if any. Will be prepended as `LlmMessage::System`.
    pub system_prompt: Option<String>,
    /// The assembled messages ready for the LLM.
    pub messages: Vec<LlmMessage>,
}

/// Context provided to a prompt assembly strategy.
///
/// Carries everything a strategy needs to produce an assembled prompt.
#[derive(Debug)]
pub struct AssemblyContext<'a> {
    /// The full conversation history for this session.
    pub history: &'a [ChatEntry],
    /// Tool definitions available for this session.
    pub tools: &'a [ToolDefinition],
    /// The name of the model being used.
    pub model_name: &'a str,
    /// The session this assembly is for.
    pub session_id: &'a SessionId,
    /// Tokens reserved for TOP/BOTTOM pinned entries that the actor will
    /// re-inject after the strategy runs. Budget-based strategies should
    /// reduce their effective budget by this amount.
    pub budget_offset: usize,
}

/// Trait for prompt assembly strategies.
///
/// Each strategy receives raw history and produces the final LLM-ready output
/// (system prompt + messages). Strategies are black boxes — they own their own
/// internal logic and state.
#[async_trait]
pub trait PromptAssembly: Send + Sync {
    /// Assemble a prompt from the given context.
    ///
    /// # Errors
    ///
    /// Returns an error if assembly fails (e.g., token estimation overflow).
    async fn assemble(
        &self,
        context: &AssemblyContext<'_>,
    ) -> Result<AssembledPrompt, Report<PromptAssemblyError>>;

    /// The name of this strategy, for debugging.
    fn name(&self) -> &'static str;
}

/// Factory for creating prompt assembly strategies by ID.
pub trait StrategyFactory: Send + Sync {
    /// Create a strategy instance for the given ID.
    ///
    /// Returns `None` if the ID is not recognized.
    ///
    /// # Errors
    ///
    /// Returns an error if strategy creation fails.
    fn create(
        &self,
        id: &PromptStrategyId,
        token_budget: usize,
    ) -> Result<Box<dyn PromptAssembly>, Report<PromptAssemblyError>>;

    /// The name of this factory, for debugging.
    fn name(&self) -> &'static str;
}

/// Metadata about an available prompt assembly strategy.
///
/// Used by the picker UI to display and select strategies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyInfo {
    /// The unique strategy identifier.
    pub id: PromptStrategyId,
    /// Human-readable display name.
    pub name: String,
    /// Short description of what the strategy does.
    pub description: String,
}

/// Discovers available prompt assembly strategies.
///
/// Returns metadata about each strategy for the picker UI.
/// This trait is separate from [`StrategyFactory`] which handles
/// creating strategy instances — discovery is purely about listing
/// what is available.
pub trait StrategyDiscovery: Send + Sync {
    /// Returns all available strategies with their metadata.
    fn list(&self) -> Vec<StrategyInfo>;

    /// The name of this discovery implementation, for debugging.
    fn name(&self) -> &'static str;
}
