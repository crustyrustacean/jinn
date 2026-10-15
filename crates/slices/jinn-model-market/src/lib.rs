//! The model-market slice — an OpenRouter-only model picker with cost
//! projection and Epoch ECI quality data.
//!
//! `<leader>so` opens a centered full-table popup (dynamic scope
//! [`jinn_model_market_msg::market_ui_scope`]): one row per text-output
//! OpenRouter listing, columns for prices, a render-time cost projection
//! under the active mix preset, and the Epoch Capabilities Index joined
//! by display name. ENTER switches the session model through the same
//! `ProviderSwitch` crossing the provider picker publishes.
//!
//! Two cells, two owners. [`jinn_model_market_msg::MarketDataState`] is
//! written only by the slice's actors (fetch, parse, cache — nothing
//! blocking ever runs on the key thread);
//! [`jinn_model_market_msg::MarketUiState`] is written only by the sync
//! side (route actions, the input hook, the render pass). The opener
//! resets the UI cell and publishes `FetchModels`; the actor serves warm
//! cache or network.
//!
//! `lib.rs` declares every module of the slice; the territories that
//! fill them own whole files and never touch this one:
//!
//! - `routes` — the opener row, every in-popup key, the filter hook
//! - `view` — the overlay render pass (table, status line, footer)
//! - `ui_state` — pure helpers over the two cells
//! - `format` — display formatting shared by view and tests
//! - `seed` — the compiled-in epoch CSVs (T2)
//! - `epoch`, `epoch_cache`, `epoch_actor` — epoch download/parse/join (T2)
//! - `eci_aliases` — the compiled-in name-join alias table (T2)
//! - `or_models`, `or_endpoints`, `market_cache`, `rows` — the
//!   OpenRouter fetch/cache/row-build layer (T3)
//! - `openrouter_actor` — the models/endpoints actor (T3)
//! - `help` — the `?` help card (T4)

pub mod format;
pub mod routes;
pub mod ui_state;
pub mod view;

// T2 — epoch data territory. Stubs; T2 fills the whole files.
pub mod eci_aliases;
pub mod epoch;
pub mod epoch_actor;
pub mod epoch_cache;
pub mod seed;

// T3 — OpenRouter data + caching territory. Stubs; T3 fills the whole files.
pub mod market_cache;
pub mod openrouter_actor;
pub mod or_endpoints;
pub mod or_models;
pub mod rows;

// T4 — help card + attribution territory. Stubs; T4 fills the whole files.
pub mod help;

use jinn_slices::SliceHost;

pub use jinn_model_market_msg::market_data_slot;
pub use jinn_model_market_msg::market_ui_scope;
pub use jinn_model_market_msg::market_ui_slot;

/// Activates the model-market slice: resolves both cells from the
/// catalog, registers the centered overlay geometry + view, and attaches
/// every route row and the filter's input hook.
///
/// Neither cell is minted here: the shared cell catalog
/// (`jinn_cell_catalog::register_all_cells`) registers every slice cell
/// in one place, so this resolves the handles the same way every other
/// consumer does.
///
/// # Panics
///
/// Panics if the catalog has not run — the overlay would otherwise
/// render against absent cells and paint nothing.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(host: &mut SliceHost<'_, jinn_slices::RenderFacts>) {
    let data = host
        .slices()
        .reader::<jinn_model_market_msg::MarketDataState>(&market_data_slot())
        .expect("the cell catalog registers the model-market data slot before any slice activates");
    let ui = host
        .slices()
        .reader::<jinn_model_market_msg::MarketUiState>(&market_ui_slot())
        .expect("the cell catalog registers the model-market UI slot before any slice activates");

    host.register_overlay(
        market_ui_scope(),
        std::sync::Arc::new(view::market_overlay_rect),
    );
    host.register_overlay_selectable(&market_ui_scope());
    host.register_overlay_slot(market_ui_scope(), market_ui_slot());
    host.register_overlay_view(
        market_ui_scope(),
        std::sync::Arc::new(view::render_market_popup),
    );
    routes::attach_market_rows(host.key_routes(), &ui, &data);
    routes::register_market_input_hook(host.key_routes(), &ui, &data);
}
