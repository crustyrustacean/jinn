//! The preferences slice — persistence for `jinn.toml` and `state.toml`.
//!
//! [`PreferencesActor`] owns `AppState.frontend.preferences`: it applies
//! `UpdatePreferences` diffs, saves `jinn.toml`, and writes the field inline.
//! [`AppStateActor`] owns `state.toml` persistence: it applies
//! `UpdateAppState` diffs and syncs the frontend theme, sidebar, and persona
//! fields inline. Both spawn at slice activation and declare their handled
//! schemas through trouper's `.handles` route registration. Their file schemas,
//! storage traits, and protocol types live in the kernel-free
//! `jinn-preferences-config` crate.

pub mod app_state_actor;
mod preferences_actor;
#[cfg(test)]
mod preferences_bus_tests;

pub use app_state_actor::AppStateActor;
pub use preferences_actor::PreferencesActor;

use jinn_domain::common::state::State;
use jinn_slices::SliceHost;

/// Activates the preferences slice's two persistence actors on the system's
/// trouper runtime.
///
/// The spawns are the readiness point: this function must complete before
/// anything publishes `EnvironmentLoaded`, whose handlers may emit
/// `UpdateAppState` or `UpdatePreferences` during first boot.
pub fn activate(
    _host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    system: &trouper::system::ActorSystem,
    services: jinn_domain::Services,
    state: State,
) {
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
