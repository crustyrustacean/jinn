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
pub mod session_picker_actions;
pub mod session_picker_render;
pub mod session_picker_routes;
pub mod session_store_actor;
pub mod sqlite;

// The search/transcript data model belongs to the store family's msg crate
// because the kernel's store seam consumes it too. Re-exported as this slice's
// vocabulary surface.
pub use jinn_session_store_msg as session_search;

use jinn_domain::Services;
use jinn_domain::common::state::State;
use jinn_slices::SliceHost;
use jinn_slices::cell::TypedCell;
use trouper::actor::ActorPath;

/// Handles returned when the session-store slice is activated.
pub struct SessionStoreHandles {
    /// Path of the store-owned session actor.
    pub session_store: ActorPath,
    /// The session picker's cell, so the actor's history read can install rows
    /// without reaching into the kernel.
    pub session_picker_cell: TypedCell<jinn_session_store_msg::SessionPickerState>,
}

/// Activates the session-store actor over the shared application state.
///
/// The actor's typed subscriptions are installed by the spawn call before it
/// returns, so messages published after activation cannot race actor startup.
///
/// # Panics
///
/// Panics if actor spawn fails, or if the session picker slot is already
/// taken. Both are composition errors — a double activation, or a slice
/// registered twice — and must abort launch rather than run with sessions
/// silently unpersisted.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: a double activation must abort launch, not run degraded"
)]
pub fn activate(services: &Services, state: State) -> SessionStoreHandles {
    // The picker's cell is minted before the actor spawn: the actor's history
    // read publishes loaded rows into it, so it needs a handle at construction.
    let session_picker_cell = services
        .slices
        .register(
            jinn_session_store_msg::session_picker_slot(),
            jinn_session_store_msg::SessionPickerState::default(),
        )
        .expect("session picker slot is registered exactly once at wiring");

    let session_store = session_store_actor::SessionStoreActor::spawn(
        &services.trouper_system,
        session_store_actor::SessionStoreActorDeps {
            services: services.clone(),
            state,
            session_picker_cell: session_picker_cell.clone(),
        },
    );

    SessionStoreHandles {
        session_store,
        session_picker_cell,
    }
}

/// Attaches the session picker's overlay, keys, and filter hook.
///
/// Split from [`activate`]: the cell is minted there (the store actor's
/// history read publishes into it) while the overlay, renderer, and key rows
/// need a [`SliceHost`], which composition owns.
///
/// # Panics
///
/// Panics if the slot is already registered — double activation is a wiring
/// bug.
pub fn activate_session_picker(
    host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    cell: &TypedCell<jinn_session_store_msg::SessionPickerState>,
) {
    let scope = jinn_session_store_msg::session_picker_scope();
    host.register_overlay(
        scope.clone(),
        std::sync::Arc::new(session_picker_render::session_picker_overlay_rect),
    );
    host.register_overlay_selectable(&scope);
    host.register_overlay_slot(scope.clone(), jinn_session_store_msg::session_picker_slot());
    host.register_overlay_view(
        scope,
        std::sync::Arc::new(session_picker_render::render_session_picker),
    );

    session_picker_routes::attach_session_picker_rows(host.key_routes(), cell);
    session_picker_routes::register_session_picker_input_hook(host.key_routes(), cell);
}

#[cfg(test)]
mod search_index_actor_tests;
#[cfg(test)]
mod session_picker_tests;
pub mod session_picker_viewport;
#[cfg(test)]
mod session_store_actor_tests;
#[cfg(test)]
mod sqlite_tests;
