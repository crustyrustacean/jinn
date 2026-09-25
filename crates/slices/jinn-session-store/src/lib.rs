//! The session-store slice — session persistence, restoration, archiving, and
//! the FTS search index.
//!
//! Owns the SQLite-backed [`SqliteSessionStore`] implementation, the schema
//! migrator, the store-owned session actor, and the background search-index
//! maintenance actor that drains the durable `fts_dirty` marker table into the
//! `session_fts` index. Live reconstruction and snapshots come from
//! `jinn-session-state`; the shared `SessionStore` service seam remains in the
//! kernel services layer.

pub mod migrator;
pub mod search_index_actor;
pub mod session_store_actor;
pub mod sqlite;

// The search/transcript data model belongs to the store family's msg crate
// because the kernel's store seam consumes it too. Re-exported as this slice's
// vocabulary surface.
pub use jinn_session_store_msg as session_search;

use jinn_domain::Services;
use jinn_domain::common::state::State;
use trouper::actor::ActorPath;

/// Handles returned when the session-store slice is activated.
pub struct SessionStoreHandles {
    /// Path of the store-owned session actor.
    pub session_store: ActorPath,
}

/// Activates the session-store actor over the shared application state.
///
/// The actor's typed subscriptions are installed by the spawn call before it
/// returns, so messages published after activation cannot race actor startup.
///
/// # Panics
///
/// Panics if actor spawn fails. A failed spawn is a composition error and must
/// abort launch rather than run with sessions silently unpersisted.
pub fn activate(services: &Services, state: State) -> SessionStoreHandles {
    let session_store = session_store_actor::SessionStoreActor::spawn(
        &services.trouper_system,
        session_store_actor::SessionStoreActorDeps {
            services: services.clone(),
            state,
            session_cap: jinn_domain::common::tcaps::mint::mint_session_cap(),
            frontend_cap: jinn_domain::common::tcaps::mint::mint_frontend_cap(),
        },
    );

    SessionStoreHandles { session_store }
}

#[cfg(test)]
mod search_index_actor_tests;
#[cfg(test)]
mod session_store_actor_tests;
#[cfg(test)]
mod sqlite_tests;
