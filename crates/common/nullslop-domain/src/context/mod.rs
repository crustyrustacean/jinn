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

pub mod actor;
pub mod prompt_scan;
pub mod strategy;

use std::path::PathBuf;
use std::sync::Arc;

use crate::actor::{Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink};
use crate::actor_host::{ActorSpawnResult, spawn_actor};
use nullslop_component::State;

pub use nullslop_protocol::PromptStrategyId;
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
    StrategyFactory, StrategyInfo, StrategySessionData,
};

/// Spawns the context actor on the given tokio runtime.
///
/// The context actor handles prompt assembly, strategy management, pinning, and templates.
pub fn spawn_context_actor(
    state: State,
    strategy_factory: Box<dyn StrategyFactory>,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<actor::ContextDirectMsg>, ActorSpawnResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<actor::ContextDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("context", sink);
    ctx.set_data(state);
    ctx.set_data(strategy_factory);
    let actor = actor::PromptAssemblyActor::activate(&mut ctx);
    let result = spawn_actor("context", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
}

/// Spawns the prompt scan actor on the given tokio runtime.
///
/// The prompt scan actor scans and reloads prompt templates from the given directory.
pub fn spawn_prompt_scan_actor(
    prompts_dir: PathBuf,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<prompt_scan::PromptScanDirectMsg>, ActorSpawnResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<prompt_scan::PromptScanDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("prompt-scan", sink);
    ctx.set_data(prompts_dir);
    let actor = prompt_scan::PromptScanActor::activate(&mut ctx);
    let result = spawn_actor("prompt-scan", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
}
