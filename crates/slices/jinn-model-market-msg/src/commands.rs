//! Model-market commands and events.
//!
//! The sync side (route actions) publishes commands; the actors publish
//! events and write [`MarketDataState`](crate::MarketDataState). Nothing
//! here carries cell handles or app state — pure data.

use serde::{Deserialize, Serialize};

use jinn_slices::BusMessage;

use crate::epoch::EpochSource;

/// Fetch the OpenRouter listings table, warm-cache-or-network.
///
/// Published by the opener. The actor serves a fresh disk/memory cache
/// without an outbound request and a stale or absent cache with one.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Fetch the OpenRouter models table, serving from cache when fresh.")]
pub struct FetchModels;

impl BusMessage for FetchModels {}

/// Fetch one model's per-endpoint detail rows.
///
/// Published by the `j`/`k` navigation actions when the highlighted row
/// has no detail yet. `seq` is the UI cell's `fetch_seq` at publish
/// time; the actor drops the fetch when a newer sequence arrived during
/// its debounce window.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Fetch one model's endpoint detail, debounced by sequence.")]
pub struct FetchEndpoints {
    /// Full model id (`openrouter/{author}/{slug}[:variant]`).
    pub model_id: String,
    /// The UI cell's fetch sequence when this request was minted.
    pub seq: u64,
}

impl BusMessage for FetchEndpoints {}

/// Re-download the epoch zip and re-extract the three CSVs.
///
/// `<c-r>` publishes this; on total failure (no network, first run) the
/// actor emits [`RefreshFailed`] and the seed data stands.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Re-download and re-extract the epoch benchmark data.")]
pub struct RefreshEpoch;

impl BusMessage for RefreshEpoch {}

/// Drop ALL cached market data and refetch everything.
///
/// `<c-r>`'s real work: the OR models cache (memory + disk), the
/// endpoint-detail cache, and the epoch cache dir all go, then epoch,
/// models, and endpoints refetch. Refresh means refresh — a later step
/// failing does not roll back the earlier drops.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Drop every market cache and refetch models, endpoints, and epoch data.")]
pub struct RefreshAll;

impl BusMessage for RefreshAll {}

/// The models table landed in the data cell.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "The OpenRouter models table was written to the market data cell.")]
pub struct ModelsLoaded {
    /// How many listings the table now holds.
    pub count: u32,
    /// `true` when a warm cache served the load (no network).
    pub from_cache: bool,
}

impl BusMessage for ModelsLoaded {}

/// One model's endpoint detail landed in the data cell.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A model's endpoint detail was written to the market data cell.")]
pub struct EndpointsLoaded {
    /// The model id the detail belongs to.
    pub model_id: String,
}

impl BusMessage for EndpointsLoaded {}

/// Epoch data landed in the data cell, with the join's coverage.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "Epoch capability data was written to the market data cell.")]
pub struct EpochLoaded {
    /// Where the data came from — the shipped seed or a download.
    pub source: EpochSource,
    /// How many ECI names the index holds.
    pub matched: u32,
    /// How many rows the source file carried (parse drops included).
    pub total: u32,
}

impl BusMessage for EpochLoaded {}

/// A refresh step failed. Earlier drops are not rolled back; the status
/// line carries `message` as the degraded-mode note.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A market refresh step failed; stale data stands.")]
pub struct RefreshFailed {
    /// Which step failed (`models`, `endpoints`, `epoch`).
    pub what: String,
    /// The failure detail for the status line.
    pub message: String,
}

impl BusMessage for RefreshFailed {}
