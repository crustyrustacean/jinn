//! The workflows slice — the `<leader>sw` picker and the workflows scan.
//!
//! Owns `workflow.md` frontmatter parsing ([`frontmatter`]), the directory
//! scan ([`scan`], exported for the session-init discovery worker to
//! delegate to), and the workflows picker itself. Portable workflow values
//! and crossing contracts live in `jinn-workflows-msg`.
//!
//! The picker is fully slice-owned: its state is a cell, its keys are route
//! rows attached at activation, and its renderer reads the cell. It reaches
//! app state through the same `as_any_mut` seam every other picker uses, so
//! the kernel holds no workflow-picker scope, keybind, or identifier. The
//! rows come from the session's discovered set (written by the session-init
//! worker) and repaint when a `WorkflowsLoaded` broadcast arrives, via this
//! slice's republisher actor.

pub mod frontmatter;
pub mod scan;
pub mod workflow_picker_actions;
pub mod workflow_picker_render;
pub mod workflow_picker_republisher_actor;
pub mod workflow_picker_routes;

pub use frontmatter::{AdmissionError, WorkflowFrontmatter, parse_workflow_frontmatter};
pub use scan::{WORKFLOW_FILE, scan_workflows};
pub use workflow_picker_actions::{highlighted_name, open};
pub use workflow_picker_render::{render_workflow_picker, workflow_picker_overlay_rect};
pub use workflow_picker_republisher_actor::WorkflowPickerRepublisherActor;
pub use workflow_picker_routes::{
    WORKFLOW_PICKER_BINDINGS, attach_workflow_picker_rows, register_workflow_picker_input_hook,
    republish_from_discovery,
};
// The picker's identity lives in the `-msg` crate beside the state it
// names; re-exported so slice consumers import it from here like the
// skills slice's `skill_picker_scope`.
pub use jinn_workflows_msg::workflows_picker_scope as workflow_picker_scope;

/// Activates the slice: registers the picker's overlay, attaches its route
/// rows, and spawns its republisher actor.
///
/// The picker's cell is not minted here: the shared cell catalog
/// (`jinn_cell_catalog::register_all_cells`) registers every slice cell in
/// one place, before any slice activates, so this resolves the handle the
/// same way every other consumer does.
///
/// # Panics
///
/// Panics if the catalog has not run — the overlay would resolve no state
/// and render nothing, and the input hook would swallow keystrokes.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(host: &mut jinn_slices::SliceHost<'_, jinn_slices::RenderFacts>) {
    let cell = host
        .slices()
        .reader::<jinn_workflows_msg::WorkflowPickerState>(
            &jinn_workflows_msg::workflows_picker_slot(),
        )
        .expect("the cell catalog registers the workflow picker slot before any slice activates");
    let scope = workflow_picker_scope();
    host.register_overlay(
        scope.clone(),
        std::sync::Arc::new(workflow_picker_overlay_rect),
    );
    host.register_overlay_selectable(&scope);
    host.register_overlay_slot(scope.clone(), jinn_workflows_msg::workflows_picker_slot());
    host.register_overlay_view(scope, std::sync::Arc::new(render_workflow_picker));

    // The picker's keys, and the filter's input hook, are the slice's own.
    attach_workflow_picker_rows(host.key_routes(), &cell);
    register_workflow_picker_input_hook(host.key_routes(), &cell);

    // Repaint the picker when a workflow scan reports new workflows.
    // Spawned here because this is the slice that owns the picker, and
    // before any scan is published so no result can slip past the
    // subscription.
    //
    // Built with the explicit builder rather than `host.spawn_service`,
    // which omits `.handles(...)`: the actor would spawn and stay
    // subscribed to nothing, so a rescan would update the session but
    // never the open menu.
    trouper::builder::spawn_service_builder::<
        workflow_picker_republisher_actor::WorkflowPickerRepublisherActor,
    >(host.system())
    .at(trouper::actor::ActorPath::new(
        workflow_picker_republisher_actor::WorkflowPickerRepublisherActor::PATH.to_owned(),
    ))
    .start_with(move || {
        Box::pin(async move {
            Ok(
                workflow_picker_republisher_actor::WorkflowPickerRepublisherActor::new(
                    cell.clone(),
                ),
            )
        })
    })
    .handles::<jinn_workflows_msg::WorkflowsLoaded>()
    .start();
}

#[cfg(test)]
mod workflow_picker_behavior_tests;
