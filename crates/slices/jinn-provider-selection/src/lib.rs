//! Provider-selection slice.
//!
//! Owns the provider-selection family end to end: the `ProviderActor`
//! (switch, refresh, picker loads, endpoint fetch) and `DiscoverActor`
//! (models.dev-enriched discovery), the crossing contracts
//! ([`jinn_provider_selection_msg`]), and the provider cell — the
//! shared model-cache + endpoint-fetch payload kernel-resident readers
//! consume through `AppState::provider_state`.
//!
//! It also hosts the **reasoning-effort picker**: the menu's state is a slice
//! cell, its keys are route rows this slice attaches, and its drawing reads
//! only that cell. The kernel and the TUI name no reasoning-effort picker at
//! all.
//!
//! The provider/endpoint picker `SelectionState`s stay on the kernel's
//! `FrontendState` (the picker host lens lends them from `&AppState`).

pub mod attachment_gate;
pub mod discover_actor;
pub mod endpoint_loader;
pub mod entries;
pub mod loader;
pub mod provider_actor;
mod reasoning_picker_actions;
pub mod reasoning_picker_render;
mod reasoning_picker_routes;
#[cfg(test)]
mod reasoning_picker_tests;
mod reasoning_picker_viewport;

use jinn_domain::Services;
use jinn_domain::common::state::State;
use jinn_provider_selection_msg::ProviderCell;
use jinn_slices::SliceHost;
use trouper::actor::ActorPath;

pub use jinn_provider_selection_msg::reasoning_picker_scope;
pub use reasoning_picker_routes::open_from_scope as open_reasoning_picker_from_scope;

/// The handles `activate` returns to composition.
pub struct ProviderSelectionHandles {
    /// The provider cell this slice minted. Wiring threads it to the
    /// boot slice (its provider-init actor writes the disk-loaded
    /// model cache through the same cell).
    pub provider_cell: jinn_slices::TypedCell<ProviderCell>,
    /// The discover actor's path.
    pub discover: ActorPath,
    /// The provider actor's path.
    pub provider: ActorPath,
}

#[expect(
    clippy::panic,
    reason = "bootstrap assertion: a double activation must abort launch, not run degraded"
)]
/// Activates the slice over a composition-owned [`SliceHost`]: mints
/// the provider cell, spawns the discover + provider actors (in that
/// order — the wiring order the kernel spawn block used), and returns
/// the handles.
///
/// # Panics
///
/// Panics if the cell slot is already taken or an actor spawn fails —
/// both mean a composition bug (double activation).
pub fn activate(
    host: &mut jinn_slices::SliceHost<'_, jinn_slices::RenderFacts>,
    services: &Services,
    state: State,
) -> ProviderSelectionHandles {
    let provider_cell = host
        .register_cell(
            jinn_provider_selection_msg::provider_state_slot(),
            ProviderCell::default(),
        )
        .unwrap_or_else(|e| panic!("provider-selection activate: provider cell slot taken: {e:?}"));

    let deps = jinn_domain::common::actor_deps::ActorDeps {
        services: services.clone(),
    };
    let discover = discover_actor::DiscoverActor::spawn(
        &services.trouper_system,
        discover_actor::DiscoverActorDeps {
            deps: deps.clone(),
            state: state.clone(),
        },
    );
    let provider = provider_actor::ProviderActor::spawn(
        &services.trouper_system,
        provider_actor::ProviderActorDeps {
            deps,
            state,
            provider_cell: provider_cell.clone(),
        },
    );

    ProviderSelectionHandles {
        provider_cell,
        discover,
        provider,
    }
}

/// Registers the reasoning-effort picker: its cell, its overlay, its keys,
/// and its filter hook.
///
/// Split from [`activate`] because the picker is a menu, not a service: it
/// spawns no actor and needs no services, so composition calls this from the
/// same slice host right after `activate` (or in its own activation step, if
/// this slice's other pickers move in later).
///
/// # Panics
///
/// Panics if the picker slot is already registered — double activation is a
/// wiring bug.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate_picker(host: &mut SliceHost<'_, jinn_slices::RenderFacts>) {
    let cell = host
        .register_cell(
            jinn_provider_selection_msg::reasoning::reasoning_picker_slot(),
            jinn_provider_selection_msg::ReasoningPickerState::default(),
        )
        .expect("reasoning picker slot is registered exactly once at wiring");

    let scope = reasoning_picker_scope();
    host.register_overlay(
        scope.clone(),
        std::sync::Arc::new(reasoning_picker_render::reasoning_picker_overlay_rect),
    );
    host.register_overlay_selectable(&scope);
    host.register_overlay_slot(
        scope.clone(),
        jinn_provider_selection_msg::reasoning::reasoning_picker_slot(),
    );
    host.register_overlay_view(
        scope,
        std::sync::Arc::new(reasoning_picker_render::render_reasoning_picker),
    );

    // The picker's keys, and the filter's input hook, are this slice's own.
    reasoning_picker_routes::attach_reasoning_picker_rows(host.key_routes(), &cell);
    reasoning_picker_routes::register_reasoning_picker_input_hook(host.key_routes(), &cell);
}
