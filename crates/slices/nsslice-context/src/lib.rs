//! Context slice — prompt assembly actor and prompt scan actor.
//!
//! - **Context actor** — manages prompt assembly, strategy management, pinning, and templates
//! - **Prompt scan actor** — scans and reloads prompt templates

pub mod actor;
pub mod prompt_scan;

use std::path::PathBuf;
use std::sync::Arc;

use nullslop_actor::{Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink};
use nullslop_actor_host::{spawn_actor, ActorSpawnResult};
use nullslop_component::State;
use nsslice_context_protocol::StrategyFactory;

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
    let result = spawn_actor(
        "context",
        actor,
        &actor_ref,
        rx,
        ctx,
        handle,
    );
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
    let result = spawn_actor(
        "prompt-scan",
        actor,
        &actor_ref,
        rx,
        ctx,
        handle,
    );
    (actor_ref, result)
}
