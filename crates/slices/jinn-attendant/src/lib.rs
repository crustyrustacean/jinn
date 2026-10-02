//! The attendant slice — sessions that watch a parent and re-run on its behalf.
//!
//! An attendant is a peer of its parent: empty history, the parent's
//! environment, and run parameters. This slice owns the trigger — when a
//! parent's turn completes successfully, every loaded attendant with
//! `ParentCompleted` pointing at it re-runs — and the seed/reset/re-run
//! behaviors that shape each run.

pub mod activation;
pub(crate) mod properties_help_card;
pub mod properties_overlay;
pub(crate) mod properties_permissions;
pub(crate) mod properties_save;
pub mod report_picker_actions;
pub mod report_picker_render;
pub mod report_picker_routes;
pub mod report_picker_viewport;
pub mod rerun;
pub mod saved_create;
pub mod saved_entry;
pub mod saved_picker_actions;
pub mod saved_picker_render;
pub mod saved_picker_routes;
pub mod saved_picker_viewport;
pub mod section_rows;
pub(crate) mod seed_template_overlay;
pub mod trigger_actor;

#[cfg(test)]
mod activation_tests;

#[cfg(test)]
mod properties_overlay_tests;

#[cfg(test)]
mod properties_render_tests;

#[cfg(test)]
mod report_picker_tests;

#[cfg(test)]
mod rerun_tests;

#[cfg(test)]
mod saved_entry_tests;

#[cfg(test)]
mod save_tests;

#[cfg(test)]
mod saved_picker_tests;

#[cfg(test)]
mod trigger_actor_tests;

use jinn_kernel::common::state::State;
use jinn_slices::SliceHost;

/// The report-history picker's opener, as a dispatchable action.
///
/// The attendants section's `s` key needs to open a picker that belongs to
/// this slice; handing the sidebar this closure keeps the dependency honest
/// in both directions (the `task_list_picker_opener` precedent).
#[must_use]
pub fn report_picker_opener() -> jinn_slices::route::ActionFn {
    report_picker_routes::report_picker_opener()
}

/// Activates the attendant slice: spawns the trigger actor.
///
/// The actor's `TurnCompleted` subscription is the readiness point — after
/// this call resolves, a published completion cannot be missed.
pub fn activate(
    host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    state: State,
    services: jinn_kernel::Services,
) {
    trigger_actor::AttendantTriggerActor::spawn(
        host.system(),
        trigger_actor::AttendantTriggerActorDeps { services, state },
    );
    activate_properties(host);
    activate_report_picker(host);
    activate_saved_picker(host);
}

/// Registers the saved-attendants picker: overlay geometry, view, slot, the
/// navigation and filter rows, and the input hook.
///
/// # Panics
///
/// Panics if the cell catalog has not run — the picker would render against
/// an absent cell and paint nothing.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate_saved_picker(host: &mut SliceHost<'_, jinn_slices::RenderFacts>) {
    let cell = host
        .slices()
        .reader::<jinn_attendant_msg::AttendantSavedPickerState>(
            &jinn_attendant_msg::attendant_saved_picker_slot(),
        )
        .expect("the cell catalog registers the saved-attendants slot before any slice activates");

    let scope = jinn_attendant_msg::attendant_saved_picker_scope();
    host.register_overlay(
        scope.clone(),
        std::sync::Arc::new(saved_picker_render::saved_picker_overlay_rect),
    );
    host.register_overlay_selectable(&scope);
    host.register_overlay_slot(
        scope.clone(),
        jinn_attendant_msg::attendant_saved_picker_slot(),
    );
    host.register_overlay_view(
        scope,
        std::sync::Arc::new(saved_picker_render::render_saved_picker),
    );

    saved_picker_routes::attach_saved_picker_rows(host.key_routes(), &cell);
    saved_picker_routes::register_saved_picker_input_hook(host.key_routes(), &cell);
}

/// Registers the report-history picker: overlay geometry, view, slot, the
/// navigation and filter rows, and the input hook.
///
/// # Panics
///
/// Panics if the cell catalog has not run — the picker would render against
/// an absent cell and paint nothing.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate_report_picker(host: &mut SliceHost<'_, jinn_slices::RenderFacts>) {
    let cell = host
        .slices()
        .reader::<jinn_attendant_msg::AttendantReportPickerState>(
            &jinn_attendant_msg::attendant_report_picker_slot(),
        )
        .expect("the cell catalog registers the report picker slot before any slice activates");

    let scope = jinn_attendant_msg::attendant_report_picker_scope();
    host.register_overlay(
        scope.clone(),
        std::sync::Arc::new(report_picker_render::report_picker_overlay_rect),
    );
    host.register_overlay_selectable(&scope);
    host.register_overlay_slot(
        scope.clone(),
        jinn_attendant_msg::attendant_report_picker_slot(),
    );
    host.register_overlay_view(
        scope,
        std::sync::Arc::new(report_picker_render::render_report_picker),
    );

    report_picker_routes::attach_report_picker_rows(host.key_routes(), &cell);
    report_picker_routes::register_report_picker_input_hook(host.key_routes(), &cell);
}

/// Registers the properties popup: overlay geometry, views, slot, rows, and
/// the template editor's input hook.
///
/// Two scopes share the one cell: the navigation-only properties form and
/// the capturing seed-template editor pushed by the form's `i` row. Both
/// scopes get overlay geometry, a selectable view (only the top scope's
/// overlay draws, so both views assemble the full form), and their own
/// rows; the input hook registers on the editor scope only, because the
/// form captures no input.
///
/// The `P` opener attaches with the sidebar's rows, in the sessions scope;
/// the popup cannot be opened from anywhere the sidebar does not offer it.
///
/// # Panics
///
/// Panics if the cell catalog has not run — the popup would render against
/// an absent cell and paint nothing.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate_properties(host: &mut SliceHost<'_, jinn_slices::RenderFacts>) {
    let cell = host
        .slices()
        .reader::<jinn_attendant_msg::AttendantPropertiesState>(
            &jinn_attendant_msg::attendant_properties_slot(),
        )
        .expect(
            "the cell catalog registers the attendant properties slot before any slice activates",
        );

    let properties_scope = jinn_attendant_msg::attendant_properties_scope();
    let editor_scope = jinn_attendant_msg::attendant_seed_template_scope();
    for scope in [&properties_scope, &editor_scope] {
        host.register_overlay(
            scope.clone(),
            std::sync::Arc::new(properties_overlay::attendant_properties_overlay_rect),
        );
        host.register_overlay_selectable(scope);
        host.register_overlay_slot(
            scope.clone(),
            jinn_attendant_msg::attendant_properties_slot(),
        );
    }
    host.register_overlay_view(
        properties_scope,
        std::sync::Arc::new(properties_overlay::render_attendant_properties),
    );
    host.register_overlay_view(
        editor_scope,
        std::sync::Arc::new(properties_overlay::render_attendant_seed_template),
    );
    properties_overlay::attach_properties_rows(host.key_routes(), &cell);
    seed_template_overlay::attach_seed_template_rows(host.key_routes(), &cell);
    seed_template_overlay::register_seed_template_input_hook(host.key_routes(), &cell);
}
