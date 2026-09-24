//! Session lifecycle slice — scripted setup, teardown, close, and cwd changes.
//!
//! This slice owns the lifecycle actor and command runner. Crossing contracts
//! and kernel-consumed leaf vocabulary remain in `jinn-session-lifecycle-msg`;
//! the kernel lifecycle intent and render handlers remain in `jinn-domain`.

pub mod command_runner;
pub mod session_lifecycle_actor;

pub use command_runner::{
    LifecycleCancelHandle, LifecycleCommandError, spawn_setup_command, spawn_teardown_command,
};
use jinn_core_types::ChatEntry;
use jinn_domain::Services;
use jinn_domain::common::state::State;
use jinn_session_lifecycle_msg::BuiltinRegistry;
use trouper::actor::ActorPath;

/// System entry shown while a setup command is running.
#[must_use]
pub fn setup_running_msg() -> ChatEntry {
    ChatEntry::system("⚙️ Running setup script...")
}

/// Handles returned when the session-lifecycle slice is activated.
pub struct SessionLifecycleHandles {
    /// Path of the lifecycle-owned session actor.
    pub lifecycle: ActorPath,
}

/// Activates the lifecycle actor over shared state and services.
///
/// # Panics
///
/// Panics if actor spawn fails. A failed spawn is a composition error and must
/// abort launch rather than run without lifecycle handling.
pub fn activate(
    services: &Services,
    state: State,
    builtin_registry: BuiltinRegistry,
    shell: String,
) -> SessionLifecycleHandles {
    let lifecycle = session_lifecycle_actor::SessionLifecycleActor::spawn(
        &services.trouper_system,
        session_lifecycle_actor::SessionLifecycleActorDeps {
            state,
            session_cap: jinn_domain::common::tcaps::mint::mint_session_cap(),
            services: services.clone(),
            builtin_registry,
            shell,
        },
    );

    SessionLifecycleHandles { lifecycle }
}

#[cfg(test)]
mod session_lifecycle_actor_tests;
