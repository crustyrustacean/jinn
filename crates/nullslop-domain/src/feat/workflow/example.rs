//! Example workflow definitions.
//!
//! Contains built-in workflow graph builders that can be registered at startup
//! and triggered via `/workflow`.

pub mod add_numbers;
pub mod branch;
pub mod dynamic;
pub mod loop_llm;
pub mod loop_pure;
pub mod pipeline;
pub mod router_demo;
pub mod summarize;

use crate::feat::workflow::workflow_registry::WorkflowRegistry;

/// Registers all example workflows into the given registry.
pub fn register(registry: &mut WorkflowRegistry) {
    add_numbers::register(registry);
    branch::register(registry);
    dynamic::register(registry);
    loop_llm::register(registry);
    loop_pure::register(registry);
    pipeline::register(registry);
    router_demo::register(registry);
    summarize::register(registry);
}
