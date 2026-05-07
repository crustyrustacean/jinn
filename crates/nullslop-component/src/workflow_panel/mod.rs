//! Workflow panel component — step list, selection, and detail view.
//!
//! Provides a first-class UI panel for navigating and inspecting workflow steps.
//! Users can browse the step list with status indicators, select steps, jump to
//! them, approve awaiting steps, and toggle a detail view showing instructions,
//! outputs, model hint, and step flags.

pub(crate) mod element;
pub(crate) mod handler;
pub mod state;

pub use element::WorkflowPanelElement;
pub use state::WorkflowPanelState;

use crate::{AppBus, AppUiRegistry};

/// Registers the workflow panel handler and UI element.
pub(crate) fn register(bus: &mut AppBus, registry: &mut AppUiRegistry) {
    handler::WorkflowPanelHandler.register(bus);
    registry.register(Box::new(WorkflowPanelElement));
}
