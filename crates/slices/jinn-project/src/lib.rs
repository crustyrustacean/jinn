//! The project slice — project-related interactive workflows.
//!
//! Owns the project-add popup's state, dynamic scope, route rows, input hook,
//! and overlay rendering. Confirming a valid path updates the open project
//! picker optimistically and persists through the configuration layer. Also
//! owns the resolver seam that decides which past sessions belong to a
//! project.

mod project_add;
pub mod project_picker_actions;
pub mod project_picker_render;
pub mod project_picker_routes;
pub mod project_picker_viewport;
pub mod scope_resolver;

use jinn_slices::SliceHost;

pub use jinn_project_msg::project_add_slot;
pub use jinn_project_msg::project_picker_scope;
pub use jinn_project_msg::project_picker_slot;
pub use project_add::intent::project_add_scope;
pub use scope_resolver::ProjectScopeResolver;

/// Activates the project slice, registering the project-add popup and the
/// project picker.
///
/// # Panics
///
/// Panics if either slot is already registered. Double activation is a
/// composition wiring error.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(host: &mut SliceHost<'_, jinn_slices::RenderFacts>) -> ProjectCells {
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

    let picker = host
        .register_cell(
            project_picker_slot(),
            jinn_project_msg::ProjectPickerState::default(),
        )
        .expect("project picker slot is registered exactly once at wiring");
    host.register_overlay(
        project_picker_scope(),
        std::sync::Arc::new(project_picker_render::project_picker_overlay_rect),
    );
    host.register_overlay_selectable(&project_picker_scope());
    host.register_overlay_slot(project_picker_scope(), project_picker_slot());
    host.register_overlay_view(
        project_picker_scope(),
        std::sync::Arc::new(project_picker_render::render_project_picker),
    );
    project_picker_routes::attach_project_picker_rows(host.key_routes(), &picker);
    project_picker_routes::register_project_picker_input_hook(host.key_routes(), &picker);
    ProjectCells {
        project_picker: picker,
    }
}

/// The cells this slice registers, so a sibling slice that must refresh an
/// open project menu can reach it without either naming the other's menu.
pub struct ProjectCells {
    /// The project picker's cell.
    pub project_picker: jinn_slices::cell::TypedCell<jinn_project_msg::ProjectPickerState>,
}
