//! Example workflow definitions.
//!
//! Contains built-in workflow graph builders that can be registered at startup
//! and triggered via `/workflow`.

pub mod add_numbers;
pub mod branch;
pub mod pipeline;
pub mod summarize;

use crate::feat::workflow::workflow_registry::WorkflowRegistry;

/// Registers all example workflows into the given registry.
pub fn register(registry: &mut WorkflowRegistry) {
    add_numbers::register(registry);
    pipeline::register(registry);
    summarize::register(registry);
    branch::register(registry);
}
