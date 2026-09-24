//! Provider-selection slice.
//!
//! Owns the provider-selection family end to end: the `ProviderActor`
//! (switch, refresh, picker loads, endpoint fetch) and `DiscoverActor`
//! (models.dev-enriched discovery), the crossing contracts
//! ([`jinn_provider_selection_msg`]), and the provider cell — the
//! shared model-cache + endpoint-fetch payload kernel-resident readers
//! (picker specs, status bar, gates) consume through
//! `AppState::provider_state`.
//!
//! The provider/endpoint picker `SelectionState`s stay on the kernel's
//! `FrontendState` (the picker host lens lends them from `&AppState`);
//! this slice's actors fill them at load time through the sanctioned
//! `FrontendCap` path.

pub mod discover_actor;
pub mod endpoint_loader;
pub mod entries;
pub mod loader;
pub mod provider_actor;

use jinn_domain::Services;
use jinn_domain::common::state::State;
use jinn_provider_selection_msg::ProviderCell;
use trouper::actor::ActorPath;

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
            session_cap: jinn_domain::common::tcaps::mint::mint_session_cap(),
            frontend_cap: jinn_domain::common::tcaps::mint::mint_frontend_cap(),
        },
    );

    ProviderSelectionHandles {
        provider_cell,
        discover,
        provider,
    }
}
