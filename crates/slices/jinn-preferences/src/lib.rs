//! The preferences slice — persistence for `jinn.toml` and `state.toml`.
//!
//! Two trouper actors, moved out of the kernel per the migration doc's
//! preferences row: [`PreferencesActor`] owns
//! `AppState.frontend.preferences` (authoritative writer — it applies
//! `UpdatePreferences` diffs, saves, and writes the field inline) and
//! [`AppStateActor`] owns `state.toml` persistence (applies
//! `UpdateAppState` diffs and syncs the frontend theme/sidebar/persona
//! fields inline). Both spawn at slice activation and declare their
//! handled schemas via trouper's `.handles` (the route registration);
//! the file schemas, storage traits, and protocol types they operate on
//! live in the kernel-free `jinn-preferences-config` crate.

pub mod app_state_actor;
mod preferences_actor;
#[cfg(test)]
mod preferences_bus_tests;
mod project_add;
mod pruner_accumulation_input;

pub use app_state_actor::AppStateActor;
pub use preferences_actor::PreferencesActor;
pub use project_add::intent::project_add_scope;
pub use project_add::intent::project_add_slot;
pub use pruner_accumulation_input::intent::pruner_accumulation_scope;
pub use pruner_accumulation_input::intent::pruner_accumulation_slot;

use jinn_domain::common::state::State;
use jinn_slices::SliceHost;

/// Activates the preferences slice: mints the project-add popup cell,
/// registers its overlay geometry/view, attaches the confirm/leave rows
/// and the editing hook, binds the `<c-n>` opener in the project
/// picker's scope, and spawns the two persistence actors on the
/// system's trouper runtime (each declaring its handled command via
/// trouper's `.handles`, which registers the delivery route).
///
/// The spawns are the readiness point: this function must complete
/// before anything publishes `EnvironmentLoaded` (whose handlers emit
/// `UpdateAppState`/`UpdatePreferences` on first boot).
///
/// # Panics
///
/// Panics if the popup slot is already registered — double activation is a
/// wiring bug.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(
    host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    system: &trouper::system::ActorSystem,
    services: jinn_domain::Services,
    state: State,
) {
    let cell = host
        .register_cell(
            project_add_slot(),
            project_add::state::ProjectAddInputState::default(),
        )
        .expect("project-add slot is registered exactly once at wiring");
    host.register_overlay(
        project_add::intent::project_add_scope(),
        std::sync::Arc::new(project_add::render::project_add_overlay_rect),
    );
    host.register_overlay_selectable(&project_add::intent::project_add_scope());
    host.register_overlay_slot(project_add::intent::project_add_scope(), project_add_slot());
    host.register_overlay_view(
        project_add::intent::project_add_scope(),
        std::sync::Arc::new(project_add::render::render_project_add_input),
    );
    project_add::intent::attach_project_add_rows(host.key_routes(), &cell);
    project_add::intent::register_project_add_input_hook(host.key_routes(), &cell);

    let pruner_cell = host
        .register_cell(
            pruner_accumulation_slot(),
            pruner_accumulation_input::state::PrunerAccumulationInputState::default(),
        )
        .expect("pruner accumulation slot is registered exactly once at wiring");
    host.register_overlay(
        pruner_accumulation_scope(),
        std::sync::Arc::new(pruner_accumulation_input::render::pruner_accumulation_overlay_rect),
    );
    host.register_overlay_slot(pruner_accumulation_scope(), pruner_accumulation_slot());
    host.register_overlay_selectable(&pruner_accumulation_scope());
    host.register_overlay_view(
        pruner_accumulation_scope(),
        std::sync::Arc::new(pruner_accumulation_input::render::render_pruner_accumulation),
    );
    pruner_accumulation_input::intent::attach_pruner_accumulation_rows(
        host.key_routes(),
        &pruner_cell,
    );
    pruner_accumulation_input::intent::register_pruner_accumulation_input_hook(
        host.key_routes(),
        &pruner_cell,
    );

    // Spawn the persistence actors (caps minted here — activation is
    // the single writer grant for each). Each spawn declares its
    // handled command via `.handles`, which registers the route.
    let prefs_path = PreferencesActor::spawn(
        system,
        services.clone(),
        state.clone(),
        jinn_domain::common::tcaps::mint::mint_frontend_cap(),
    );
    drop(prefs_path);
    let app_state_path = AppStateActor::spawn(
        system,
        services,
        state,
        jinn_domain::common::tcaps::mint::mint_frontend_cap(),
    );
    drop(app_state_path);
}
