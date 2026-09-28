//! The session-store slice — session persistence, restoration, archiving, and
//! the FTS search index.
//!
//! Owns the SQLite-backed [`SqliteSessionStore`] implementation, the schema
//! migrator, the store-owned session actor, and the background search-index
//! maintenance actor that drains the durable `fts_dirty` marker table into the
//! `session_fts` index. Live reconstruction and snapshots come from
//! `jinn-session-state`; the shared `SessionStore` service seam remains in the
//! kernel services layer.

pub mod hydrate;
pub mod hydrate_worker;
pub mod migrator;
pub mod search_index_actor;
pub mod session_entries;
#[cfg(test)]
mod session_entries_tests;
pub mod session_picker_actions;
pub mod session_picker_render;
pub mod session_picker_routes;
pub mod session_store_actor;
pub mod sqlite;

// The search/transcript data model belongs to the store family's msg crate
// because the kernel's store seam consumes it too. Re-exported as this slice's
// vocabulary surface.
pub use jinn_session_store_msg as session_search;

use jinn_kernel::Services;
use jinn_kernel::common::state::State;
use jinn_slices::SliceHost;
use jinn_slices::cell::TypedCell;
use trouper::actor::ActorPath;

/// Handles returned when the session-store slice is activated.
pub struct SessionStoreHandles {
    /// Path of the store-owned session actor.
    pub session_store: ActorPath,
}

/// Activates the session-store actor over the shared application state.
///
/// The session picker's cell is not minted here: the shared cell catalog
/// (`jinn_cell_catalog::register_all_cells`) registers every slice cell in
/// one place, and `activate` resolves the handle the actor's history read
/// publishes into.
///
/// The actor's typed subscriptions are installed by the spawn call before it
/// returns, so messages published after activation cannot race actor startup.
///
/// # Panics
///
/// Panics if actor spawn fails, or if the catalog has not run. A failed
/// spawn is a composition error and must abort launch rather than run with
/// sessions silently unpersisted; an absent cell would drop every history
/// read the actor publishes.
///
/// The returned [`SessionStoreHandles`] carries the session-store actor path.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: a broken harness must abort launch, not run degraded"
)]
pub fn activate(
    host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    services: &Services,
    state: State,
) -> SessionStoreHandles {
    // The actor's history read publishes loaded rows into this cell, so it
    // needs a handle at construction.
    let session_picker_cell = host
        .slices()
        .reader::<jinn_session_store_msg::SessionPickerState>(
            &jinn_session_store_msg::session_picker_slot(),
        )
        .expect("the cell catalog registers the session-picker slot before any slice activates");

    let session_store = session_store_actor::SessionStoreActor::spawn(
        host.system(),
        session_store_actor::SessionStoreActorDeps {
            services: services.clone(),
            state,
            session_picker_cell,
        },
    );

    SessionStoreHandles { session_store }
}

/// Attaches the session picker's overlay, keys, and filter hook.
///
/// Split from [`activate`]: the actor that publishes into the cell is
/// spawned there while the overlay, renderer, and key rows need a
/// [`SliceHost`], which composition owns.
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
mod hydrate_worker_tests;
#[cfg(test)]
mod search_index_actor_tests;
#[cfg(test)]
mod session_picker_tests;
pub mod session_picker_viewport;
#[cfg(test)]
mod session_store_actor_tests;
#[cfg(test)]
mod session_store_tests_support;
#[cfg(test)]
mod sqlite_tests;
