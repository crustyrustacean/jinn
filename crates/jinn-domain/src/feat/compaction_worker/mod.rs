//! Compaction worker - summarizes conversation history into structured checkpoints.
//!
//! Implements [`HistoryWorker`] to produce [`HistoryMutation`] batches.
//! The compaction algorithm:
//! 1. Finds the start boundary (after the last compaction summary)
//! 2. Computes a cut index respecting the token reserve
//! 3. Adjusts the cut to avoid breaking tool loop boundaries
//! 4. Calls the LLM for summarization
//! 5. Produces mutations: `SetContextOverride(ForcedExclude)` for gathered entries,
//!    `InsertEntry` for the compaction summary

pub mod algorithm;
pub mod serializer;
pub mod trigger_actor;
pub mod worker;

pub use trigger_actor::{CompactionTriggerActor, CompactionTriggerActorDeps};
pub use worker::{CompactionTrigger, CompactionWorker};

use serde::{Deserialize, Serialize};

/// Default token threshold for auto-compaction.
const DEFAULT_COMPACTION_THRESHOLD: f64 = 0.7;

/// Default number of recent tokens to reserve from compaction.
const DEFAULT_RESERVE_TOKENS: usize = 20_000;

/// Default fallback context window when the provider doesn't report one.
const DEFAULT_FALLBACK_CONTEXT_WINDOW: usize = 150_000;

/// Compaction configuration.
///
/// Serialized as `[compaction]` in `jinn.toml`.
/// Controls when and how context compaction summarizes conversation history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Provider/model for compaction summarization (e.g., "anthropic/claude-sonnet-4-20250514").
    /// Falls back to the session model if not set or if provider construction fails.
    #[serde(default)]
    pub model: Option<String>,
    /// Fraction of context window at which auto-compaction triggers (0.0–1.0).
    /// Default: 0.7 (70% of budget).
    #[serde(default = "default_compaction_threshold")]
    pub threshold: f64,
    /// Number of recent tokens to reserve from compaction.
    /// Default: 20,000.
    #[serde(default = "default_reserve_tokens")]
    pub reserve_tokens: usize,
    /// Fallback context window size when the provider doesn't report `context_length`.
    /// Used for auto-compaction threshold calculation with local models (Ollama, LM Studio).
    /// Default: 150,000.
    #[serde(default = "default_fallback_context_window")]
    pub fallback_context_window: usize,
}

fn default_compaction_threshold() -> f64 {
    DEFAULT_COMPACTION_THRESHOLD
}

fn default_reserve_tokens() -> usize {
    DEFAULT_RESERVE_TOKENS
}

fn default_fallback_context_window() -> usize {
    DEFAULT_FALLBACK_CONTEXT_WINDOW
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            model: None,
            threshold: DEFAULT_COMPACTION_THRESHOLD,
            reserve_tokens: DEFAULT_RESERVE_TOKENS,
            fallback_context_window: DEFAULT_FALLBACK_CONTEXT_WINDOW,
        }
    }
}
