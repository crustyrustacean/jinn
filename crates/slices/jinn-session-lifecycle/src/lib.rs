//! Session lifecycle slice — scripted setup, teardown, close, and cwd changes.
//!
//! This slice owns the lifecycle actor and command runner. Crossing contracts
//! and kernel-consumed leaf vocabulary remain in `jinn-session-lifecycle-msg`;
//! the kernel lifecycle intent and render handlers remain in `jinn-kernel`.

pub mod arg_input;
pub mod arg_input_render;
pub mod command_runner;
pub mod session_lifecycle_actor;
mod session_lifecycle_picker_actions;
pub mod session_lifecycle_picker_render;
mod session_lifecycle_picker_routes;
mod session_lifecycle_picker_viewport;

pub use command_runner::{
    LifecycleCancelHandle, LifecycleCommandError, spawn_setup_command, spawn_teardown_command,
};
use jinn_kernel::Services;
use jinn_kernel::common::state::State;
use jinn_session_lifecycle_msg::ArgInputState;
use jinn_session_lifecycle_msg::BuiltinRegistry;
use jinn_slices::SliceHost;

/// Activates the lifecycle actor over shared state and services.
///
/// The argument input's cell is not minted here: the shared cell catalog
/// (`jinn_cell_catalog::register_all_cells`) registers every slice cell in
/// one place, so this resolves the handle the same way every other
/// consumer does.
///
/// # Panics
///
/// Panics if actor spawn fails, or if the catalog has not run. A failed
/// spawn is a composition error and must abort launch rather than run
/// without lifecycle handling; an absent cell would leave the argument
/// overlay rendering nothing.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(
    host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    services: &Services,
    state: State,
    builtin_registry: BuiltinRegistry,
    shell: String,
) {
    let cell = host
        .slices()
        .reader::<ArgInputState>(&jinn_session_lifecycle_msg::arg_input_slot())
        .expect(
            "the cell catalog registers the lifecycle arg-input slot before any slice activates",
        );
    let geometry_cell = cell.clone();
    host.register_overlay(
        jinn_session_lifecycle_msg::arg_input_scope(),
        std::sync::Arc::new(move |area: &ratatui::layout::Rect| {
            Some(arg_input_render::arg_input_overlay_rect(
                *area,
                &geometry_cell,
            ))
        }),
    );
    host.register_overlay_slot(
        jinn_session_lifecycle_msg::arg_input_scope(),
        jinn_session_lifecycle_msg::arg_input_slot(),
    );
    host.register_overlay_view(
        jinn_session_lifecycle_msg::arg_input_scope(),
        std::sync::Arc::new(arg_input_render::render_arg_input),
    );
    host.register_overlay_selectable(&jinn_session_lifecycle_msg::arg_input_scope());
    arg_input::attach_rows(host.key_routes(), &cell);
    arg_input::register_input_hook(host.key_routes(), &cell);

    let _lifecycle = session_lifecycle_actor::SessionLifecycleActor::spawn(
        host.system(),
        session_lifecycle_actor::SessionLifecycleActorDeps {
            state,
            services: services.clone(),
            builtin_registry,
            shell,
        },
    );
}

/// Registers the session-lifecycle picker: its overlay, its keys, and
/// its filter hook.
///
/// Split from [`activate`] because the picker is a menu over configured
/// lifecycles while `activate` owns the actor that runs setup and teardown.
/// Called from composition right after `activate`.
///
/// Like [`activate`], the picker's cell is not minted here: the shared cell
/// catalog registers it in one place.
///
/// # Panics
///
/// Panics if the catalog has not run — the overlay would otherwise render
/// against an absent cell and paint nothing.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate_picker(host: &mut SliceHost<'_, jinn_slices::RenderFacts>) {
    let cell = host
        .slices()
        .reader::<jinn_session_lifecycle_msg::SessionLifecyclePickerState>(
            &jinn_session_lifecycle_msg::session_lifecycle_picker_slot(),
        )
        .expect("the cell catalog registers the lifecycle picker slot before any slice activates");

    let scope = jinn_session_lifecycle_msg::session_lifecycle_picker_scope();
    host.register_overlay(
        scope.clone(),
        std::sync::Arc::new(session_lifecycle_picker_render::session_lifecycle_picker_overlay_rect),
    );
    host.register_overlay_selectable(&scope);
    host.register_overlay_slot(
        scope.clone(),
        jinn_session_lifecycle_msg::session_lifecycle_picker_slot(),
    );
    host.register_overlay_view(
        scope,
        std::sync::Arc::new(session_lifecycle_picker_render::render_session_lifecycle_picker),
    );

    session_lifecycle_picker_routes::attach_session_lifecycle_picker_rows(host.key_routes(), &cell);
    session_lifecycle_picker_routes::register_session_lifecycle_picker_input_hook(
        host.key_routes(),
        &cell,
    );
    session_lifecycle_picker_routes::register_session_lifecycle_picker_enter_hook(
        host.key_routes(),
        &cell,
    );
}

#[cfg(test)]
mod session_lifecycle_actor_tests;
#[cfg(test)]
mod session_lifecycle_picker_tests;
