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
mod endpoint_picker_actions;
pub mod endpoint_picker_render;
mod endpoint_picker_routes;
#[cfg(test)]
mod endpoint_picker_tests;
mod endpoint_picker_viewport;
pub mod entries;
pub mod loader;
pub mod provider_actor;
pub mod provider_picker_actions;
pub mod provider_picker_render;
pub mod provider_picker_routes;
#[cfg(test)]
mod provider_picker_tests;
pub mod provider_picker_viewport;
mod reasoning_picker_actions;
pub mod reasoning_picker_render;
mod reasoning_picker_routes;
#[cfg(test)]
mod reasoning_picker_tests;
mod reasoning_picker_viewport;

use jinn_kernel::Services;
use jinn_kernel::common::state::State;
use jinn_provider_selection_msg::ProviderCell;
use jinn_slices::SliceHost;
use trouper::actor::ActorPath;

pub use jinn_provider_selection_msg::endpoint::endpoint_picker_scope;
pub use jinn_provider_selection_msg::reasoning_picker_scope;
pub use reasoning_picker_routes::open_from_scope as open_reasoning_picker_from_scope;

/// The handles `activate` returns to composition.
pub struct ProviderSelectionHandles {
    /// The provider cell this slice resolved. Wiring threads it to the
    /// boot slice (its provider-init actor writes the disk-loaded
    /// model cache through the same cell).
    pub provider_cell: jinn_slices::TypedCell<ProviderCell>,
    /// The endpoint picker's cell. The provider actor publishes each
    /// completed fetch into it and composition hands the handle back to
    /// `activate_endpoint_picker` to finish wiring.
    /// The model picker's cell, so the provider actor's loads land in the menu
    /// that shows them.
    pub provider_picker_cell:
        jinn_slices::cell::TypedCell<jinn_provider_selection_msg::ProviderPickerState>,
    pub endpoint_picker_cell:
        jinn_slices::cell::TypedCell<jinn_provider_selection_msg::endpoint::EndpointPickerState>,
    /// The discover actor's path.
    pub discover: ActorPath,
    /// The provider actor's path.
    pub provider: ActorPath,
}

#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
/// Activates the slice over a composition-owned [`SliceHost`]: resolves
/// the three cells its actors publish into, spawns the discover + provider
/// actors (in that order — the wiring order the kernel spawn block used),
/// and returns the handles.
///
/// No cell is minted here: the shared cell catalog
/// (`jinn_cell_catalog::register_all_cells`) registers every slice cell in
/// one place, so this resolves the handles the same way every other
/// consumer does.
///
/// # Panics
///
/// Panics if the catalog has not run, or an actor spawn fails — the
/// former means the picker actors would publish into absent cells, the
/// latter that composition is wired wrong.
pub fn activate(
    host: &mut jinn_slices::SliceHost<'_, jinn_slices::RenderFacts>,
    services: &Services,
    state: State,
) -> ProviderSelectionHandles {
    let provider_cell = host
        .slices()
        .reader::<ProviderCell>(&jinn_provider_selection_msg::provider_state_slot())
        .expect("the cell catalog registers the provider-state slot before any slice activates");

    // The provider actor publishes each completed fetch and each model load
    // into these two cells, so it must hold handles to the catalog's cells —
    // a clone shares the catalog's own payload.
    let endpoint_picker_cell = host
        .slices()
        .reader::<jinn_provider_selection_msg::endpoint::EndpointPickerState>(
            &jinn_provider_selection_msg::endpoint::endpoint_picker_slot(),
        )
        .expect("the cell catalog registers the endpoint-picker slot before any slice activates");
    let provider_picker_cell = host
        .slices()
        .reader::<jinn_provider_selection_msg::ProviderPickerState>(
            &jinn_provider_selection_msg::provider_picker_slot(),
        )
        .expect("the cell catalog registers the provider-picker slot before any slice activates");

    let deps = jinn_kernel::common::actor_deps::ActorDeps {
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
            endpoint_picker_cell: endpoint_picker_cell.clone(),
            provider_picker_cell: provider_picker_cell.clone(),
        },
    );

    ProviderSelectionHandles {
        provider_cell,
        provider_picker_cell,
        endpoint_picker_cell,
        discover,
        provider,
    }
}

/// Registers the model picker: its overlay, its keys, and its filter hook.
///
/// The cell itself belongs to the shared catalog; [`activate`] resolved
/// its handle so the provider actor could hold one and publish each load
/// into it, and composition threads the same handle here.
///
/// No services, no actor: the menu is pure slice state plus a bus message.
pub fn activate_provider_picker(
    host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    cell: &jinn_slices::cell::TypedCell<jinn_provider_selection_msg::ProviderPickerState>,
) {
    let cell = cell.clone();
    let scope = jinn_provider_selection_msg::provider_picker_scope();
    host.register_overlay(
        scope.clone(),
        std::sync::Arc::new(provider_picker_render::provider_picker_overlay_rect),
    );
    host.register_overlay_selectable(&scope);
    host.register_overlay_slot(
        scope.clone(),
        jinn_provider_selection_msg::provider_picker_slot(),
    );
    host.register_overlay_view(
        scope,
        std::sync::Arc::new(provider_picker_render::render_provider_picker),
    );

    provider_picker_routes::attach_provider_picker_rows(host.key_routes(), &cell);
    provider_picker_routes::register_provider_picker_input_hook(host.key_routes(), &cell);
}

/// Registers the reasoning-effort picker: its overlay, its keys,
/// and its filter hook.
///
/// Split from [`activate`] because the picker is a menu, not a service: it
/// spawns no actor and needs no services, so composition calls this from the
/// same slice host right after `activate` (or in its own activation step, if
/// this slice's other pickers move in later). Its cell belongs to the shared
/// catalog, so this resolves the handle rather than minting one.
///
/// # Panics
///
/// Panics if the catalog has not run — the menu's rows and render would
/// otherwise act on an absent cell and paint nothing.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate_picker(host: &mut SliceHost<'_, jinn_slices::RenderFacts>) {
    let cell = host
        .slices()
        .reader::<jinn_provider_selection_msg::ReasoningPickerState>(
            &jinn_provider_selection_msg::reasoning::reasoning_picker_slot(),
        )
        .expect("the cell catalog registers the reasoning-picker slot before any slice activates");

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

/// Registers the OpenRouter endpoint picker: its overlay, its keys, and its
/// filter hook.
///
/// The cell itself belongs to the shared catalog; [`activate`] resolved
/// its handle so the provider actor could hold one and publish each
/// completed fetch into it, and composition threads the same handle here.
///
/// Each picker owns a distinct scope, cell, and key set, and none of them
/// knows another exists.
pub fn activate_endpoint_picker(
    host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    cell: &jinn_slices::cell::TypedCell<jinn_provider_selection_msg::endpoint::EndpointPickerState>,
) {
    let cell = cell.clone();
    let scope = endpoint_picker_scope();
    host.register_overlay(
        scope.clone(),
        std::sync::Arc::new(endpoint_picker_render::endpoint_picker_overlay_rect),
    );
    host.register_overlay_selectable(&scope);
    host.register_overlay_slot(
        scope.clone(),
        jinn_provider_selection_msg::endpoint::endpoint_picker_slot(),
    );
    host.register_overlay_view(
        scope,
        std::sync::Arc::new(endpoint_picker_render::render_endpoint_picker),
    );

    endpoint_picker_routes::attach_endpoint_picker_rows(host.key_routes(), &cell);
    endpoint_picker_routes::register_endpoint_picker_input_hook(host.key_routes(), &cell);
}
