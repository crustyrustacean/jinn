//! The boot slice: the startup trio.
//!
//! Spawns the three startup actors — [`SystemReadyActor`] (main-thread
//! readiness signal), [`EnvInitActor`] (lazy `providers.toml` load +
//! API-key/MCP-header env resolution), and [`ProviderInitActor`]
//! (registry build, model-cache merge, `last_model` re-apply) — in the
//! same order composition spawned them before this slice existed.
//!
//! The startup tail (the `GetEnvironmentConfig` ask and the
//! `EnvironmentLoaded` publish) stays in composition
//! (`actor_wiring`): it is ordering orchestration that must run after
//! `AllActorsSpawned` fires, and composition is the spawn/drain point.
//! [`install_actors`] returns the handles the tail needs.

pub mod env_init_actor;
pub mod no_api_keys;
pub mod provider_init_actor;
pub mod system_ready_actor;

pub use env_init_actor::EnvInitActor;
pub use provider_init_actor::ProviderInitActor;
pub use system_ready_actor::SystemReadyActor;

use jinn_kernel::common::services::Services;
use jinn_kernel::common::state::State;
use trouper::actor::ActorPath;

/// The handles composition needs from the boot installation: the
/// env-init actor's path (for the startup-tail ask) and the readiness
/// receiver (the main thread blocks on it until `AllActorsSpawned`).
#[derive(Debug)]
pub struct BootHandles {
    /// The env-init actor's path — the ask target for the startup tail.
    pub env_init_path: ActorPath,
    /// Signals main-thread readiness when `AllActorsSpawned` fires.
    pub ready_rx: kanal::Receiver<()>,
}

/// Spawns the boot trio onto the trouper system and returns the
/// startup-tail handles.
///
/// Spawn order matters and matches the pre-slice wiring: system-ready →
/// env-init → provider-init. Each actor's subscription is live when its
/// spawn returns.
///
/// # Panics
///
/// Panics if any actor's path is already taken or a topic subscription
/// fails — both mean a wiring bug at composition (double install).
#[must_use]
pub fn install_actors(
    system: &trouper::system::ActorSystem,
    state: State,
    services: &Services,
    provider_cell: jinn_slices::TypedCell<jinn_provider_selection_msg::ProviderCell>,
) -> BootHandles {
    // System-ready actor: signals main thread when all actors started.
    let (ready_tx, ready_rx) = kanal::unbounded::<()>();
    let _system_ready = SystemReadyActor::spawn(
        system,
        system_ready_actor::SystemReadyActorDeps {
            deps: jinn_kernel::common::actor_deps::ActorDeps {
                services: services.clone(),
            },
            ready_tx,
        },
    );

    // Env init: config loading is deferred to the GetEnvironmentConfig
    // ask (the ONE real ask path — composition asks it in the startup
    // tail, with a mandatory timeout).
    let env_init_path = EnvInitActor::spawn(
        system,
        env_init_actor::EnvInitActorDeps {
            deps: jinn_kernel::common::actor_deps::ActorDeps {
                services: services.clone(),
            },
        },
    );

    // Provider init: on EnvironmentLoaded, builds registry, merges cache,
    // resolves last_model.
    let _provider_init = ProviderInitActor::spawn(
        system,
        provider_init_actor::ProviderInitActorDeps {
            deps: jinn_kernel::common::actor_deps::ActorDeps {
                services: services.clone(),
            },
            state,
            provider_cell,
        },
    );

    BootHandles {
        env_init_path,
        ready_rx,
    }
}
