//! Workflow management component — handles lifecycle commands for structured multi-step workflows.
//!
//! Processes commands that load, advance, jump within, and abort workflows.
//! All state mutations go through `AppState::workflow` ([`AppState`](crate::AppState)).

pub mod handler;

pub(crate) use handler::WorkflowHandler;

use crate::AppBus;
use crate::AppUiRegistry;

/// Register the workflow handler.
pub(crate) fn register(bus: &mut AppBus, _registry: &mut AppUiRegistry) {
    WorkflowHandler.register(bus);
}
