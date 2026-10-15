//! The fetched-data cell payload: everything the actors write and
//! everything the popup's table, detail pane, and status line read.

use std::sync::Arc;

use jiff::Timestamp;
use jinn_slices::SlotKey;

use crate::epoch::EciIndex;
use crate::row::MarketRow;

/// The model-market's data cell: the fetched table, the epoch index, the
/// cache stamps, and the in-flight flags.
///
/// Written only by the slice's actors. The synchronous side (route
/// actions, input hook, render pass) reads it and mutates its own
/// [`MarketUiState`](crate::MarketUiState) instead.
#[derive(Debug, Clone, Default)]
pub struct MarketDataState {
    /// One row per OpenRouter listing, empty until the first
    /// `ModelsLoaded` lands.
    pub rows: Vec<MarketRow>,
    /// The highlighted model's per-endpoint detail, once fetched.
    pub detail: Option<MarketDetail>,
    /// Parsed epoch scores keyed by ECI display name. `Arc` so an epoch
    /// refresh swaps one pointer instead of cloning the map per reader.
    pub eci: Option<Arc<EciIndex>>,
    /// When the models cache was last written, for the status line's
    /// "updated X ago" stamp.
    pub models_fetched_at: Option<Timestamp>,
    /// When the epoch data was last written.
    pub epoch_fetched_at: Option<Timestamp>,
    /// When the highlighted model's endpoints were last written.
    pub endpoints_fetched_at: Option<Timestamp>,
    /// A models fetch is in flight.
    pub models_loading: bool,
    /// An endpoint-detail fetch is in flight.
    pub endpoints_loading: bool,
    /// An epoch download/parse is in flight.
    pub epoch_loading: bool,
    /// Where the epoch data on display came from, so the user can tell
    /// the shipped snapshot from a fresh download.
    pub epoch_source: crate::epoch::EpochSource,
    /// The most recent failure, shown as the popup's degraded-mode line.
    pub last_error: Option<String>,
}

/// The highlighted model's per-endpoint detail rows.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketDetail {
    /// The full model id the detail was fetched for
    /// (`openrouter/{author}/{slug}[:variant]`).
    pub model_id: String,
    /// One row per listing endpoint.
    pub endpoints: Vec<crate::row::MarketEndpointRow>,
}

/// The data cell's slot key, registered by the cell catalog before any
/// slice activates.
#[must_use]
pub fn market_data_slot() -> SlotKey {
    SlotKey::builtin("model-market", "data")
}
