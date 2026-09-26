//! The project slice — project-related interactive workflows.
//!
//! Owns the project-add popup's state, dynamic scope, route rows, input hook,
//! and overlay rendering. Confirming a valid path updates the open project
//! picker optimistically and persists through the configuration layer. Also
//! owns the resolver seam that decides which past sessions belong to a
//! project.

mod project_add;
pub mod scope_resolver;

use jinn_slices::SliceHost;

pub use jinn_project_msg::project_add_slot;
pub use project_add::intent::project_add_scope;
pub use scope_resolver::ProjectScopeResolver;

/// Activates the project slice and registers the project-add popup.
///
/// # Panics
///
/// Panics if the project-add slot is already registered. Double activation is a
/// composition wiring error.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(host: &mut SliceHost<'_, jinn_slices::RenderFacts>) {
    let cell = host
        .register_cell(
            project_add_slot(),
            jinn_project_msg::ProjectAddInputState::default(),
        )
        .expect("project-add slot is registered exactly once at wiring");
    host.register_overlay(
        project_add_scope(),
        std::sync::Arc::new(project_add::render::project_add_overlay_rect),
    );
    host.register_overlay_selectable(&project_add_scope());
    host.register_overlay_slot(project_add_scope(), project_add_slot());
    host.register_overlay_view(
        project_add_scope(),
        std::sync::Arc::new(project_add::render::render_project_add_input),
    );
    project_add::intent::attach_project_add_rows(host.key_routes(), &cell);
    project_add::intent::register_project_add_input_hook(host.key_routes(), &cell);
}
