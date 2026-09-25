//! The preferences slice — persistence and preference-driven controls.
//!
//! [`PreferencesActor`] owns `AppState.frontend.preferences`: it applies
//! `UpdatePreferences` diffs, saves `jinn.toml`, and writes the field inline.
//! [`AppStateActor`] owns `state.toml` persistence: it applies
//! `UpdateAppState` diffs and syncs the frontend theme, sidebar, and persona
//! fields inline. The pruner-accumulation input popup is also activated here
//! because its scope and route are driven by preference-controlled pruning
//! state. The file schemas, storage traits, and protocol types used by the
//! actors live in the kernel-free `jinn-preferences-config` crate.

pub mod app_state_actor;
mod preferences_actor;
#[cfg(test)]
mod preferences_bus_tests;
mod pruner_accumulation_input;

pub use app_state_actor::AppStateActor;
pub use preferences_actor::PreferencesActor;
pub use pruner_accumulation_input::intent::pruner_accumulation_scope;
pub use pruner_accumulation_input::intent::pruner_accumulation_slot;

use jinn_domain::common::state::State;
use jinn_slices::SliceHost;

/// Activates the preferences slice's pruner-accumulation popup and two
/// persistence actors on the system's trouper runtime.
///
/// The actor spawns are the readiness point: this function must complete
/// before anything publishes `EnvironmentLoaded`, whose handlers may emit
/// `UpdateAppState` or `UpdatePreferences` during first boot.
///
/// # Panics
///
/// Panics if the pruner-accumulation slot is already registered. Double
/// activation is a wiring error.
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

    // Spawn the persistence actors. Each spawn declares its handled command
    // via `.handles`, which registers the route.
    let prefs_path = PreferencesActor::spawn(system, services.clone(), state.clone());
    drop(prefs_path);
    let app_state_path = AppStateActor::spawn(system, services, state);
    drop(app_state_path);
}
