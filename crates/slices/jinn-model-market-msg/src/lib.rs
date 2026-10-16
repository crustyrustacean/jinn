//! Model-market crossing contracts — the two cell payloads every side of
//! the slice boundary reads, plus the fetch/refresh commands and events.
//!
//! The split mirrors the slice's ownership rule: [`MarketDataState`] is
//! written only by the slice's actors (the fetched table, the epoch index,
//! the cache stamps), while [`MarketUiState`] is written only by the
//! synchronous side (route actions, the input hook, the render pass).
//! A writer on the wrong side is the sync-sibling anti-pattern; the two
//! payloads' docs name their owner so the boundary stays visible.
//!
//! The crossing-schema ids (`FetchModels`, `ModelsLoaded`, …) derive from
//! the type names, so they are part of the wire contract and must not be
//! renamed.

mod commands;
mod data_state;
mod epoch;
mod row;
mod ui_state;

pub use commands::*;
pub use data_state::{MarketDataState, MarketDetail, market_data_slot};
pub use epoch::{EciIndex, EciScore, EpochSource};
pub use row::{MarketEndpointRow, MarketRow};
pub use ui_state::{
    MarketUiState, PROJECTION_MIX, RESULTS_VIEWPORT_FALLBACK, SORT_CYCLE, SortColumn,
    market_ui_scope, market_ui_slot,
};

#[cfg(test)]
mod tests;
