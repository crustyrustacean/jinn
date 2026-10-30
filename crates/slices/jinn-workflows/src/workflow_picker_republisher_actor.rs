//! Republishes the workflows picker when a discovery scan reports new
//! workflows.
//!
//! The picker is a slice cell, so a worker holding session state cannot
//! write it directly — it holds no handle to the slice registry. Instead
//! the scan broadcasts [`WorkflowsLoaded`], and this listener, which is
//! spawned by the workflows slice with a handle to its own cell, rebuilds
//! the rows.
//!
//! Keeping the write on this side of the boundary is what makes the picker
//! slice-owned: the discovery worker publishes a resource event and knows
//! nothing about which slice renders the result.

use jinn_slices::cell::TypedCell;
use jinn_workflows_msg::WorkflowPickerState;
use jinn_workflows_msg::WorkflowsLoaded;
use trouper::actor::{MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;

/// Republishes the picker's rows whenever a workflow scan reports a result.
#[derive(Debug)]
pub struct WorkflowPickerRepublisherActor {
    /// The picker's cell, written on every scan result.
    cell: TypedCell<WorkflowPickerState>,
}

impl WorkflowPickerRepublisherActor {
    /// The registry path this actor is published under.
    pub const PATH: &'static str = "jinn.workflows.picker-republisher";

    /// Builds a republisher over the picker's own cell.
    #[must_use]
    pub fn new(cell: TypedCell<WorkflowPickerState>) -> Self {
        Self { cell }
    }
}

impl ServiceActor for WorkflowPickerRepublisherActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "ServiceActor::start is async by signature; this impl never awaits"
    )]
    async fn init(_args: &trouper::json::Json) -> Result<Self, error_stack::Report<RegistryError>> {
        // Never called: spawned via `init_with` (typed deps cannot ride
        // JSON args).
        Err(error_stack::Report::new(RegistryError::InvalidSpec)
            .attach("WorkflowPickerRepublisherActor spawns via init_with"))
    }
}

impl MsgHandler<WorkflowsLoaded> for WorkflowPickerRepublisherActor {
    /// Repaints the picker's rows when a workflow scan reports a result.
    async fn handle(&mut self, msg: &WorkflowsLoaded, _ctx: &mut MsgCtx<'_>) {
        // A failed scan reports no workflows; republishing would blank the
        // picker.
        if msg.error.is_some() {
            return;
        }
        crate::workflow_picker_routes::republish_from_discovery(&self.cell, &msg.workflows);
    }
}
