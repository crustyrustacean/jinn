//! The popup's UI cell payload: what only the open menu mutates.
//!
//! Written by the sync side only — the opener resets it, route actions
//! and the input hook move it, the render pass measures it. The actors
//! never touch it: [`MarketDataState`](crate::MarketDataState) is theirs.

use jinn_slices::SliceScopeId;
use jinn_slices::SlotKey;

/// The row count to assume before the first render pass measures the
/// popup. A fallback, not a default: the render pass overwrites it.
pub const RESULTS_VIEWPORT_FALLBACK: usize = 20;

/// The sort-column cycle order: the default first, then left-to-right
/// across the sortable columns.
pub const SORT_CYCLE: [SortColumn; 6] = [
    SortColumn::Proj,
    SortColumn::In,
    SortColumn::Out,
    SortColumn::Cache,
    SortColumn::Eci,
    SortColumn::Model,
];

/// The fixed cost projection every PROJ value is computed under, as
/// `(input tokens, cache-hit fraction, output tokens)`. Cache misses
/// bill at the prompt price (locked decision F). A constant, not an
/// option: the projection is part of the slice's contract, so every
/// reader — the table cells, the sort, the help card — names the same
/// mix.
pub const PROJECTION_MIX: (u32, f64, u32) = (10_000_000, 0.98, 50_000);

/// Which column the sort is keyed on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortColumn {
    /// Projected $/turn under [`PROJECTION_MIX`] — the default.
    #[default]
    Proj,
    /// Prompt price, $/M.
    In,
    /// Completion price, $/M.
    Out,
    /// Cached-input read price, $/M (`None` sorts last either way).
    Cache,
    /// Epoch Capabilities Index (unmatched rows sort last either way).
    Eci,
    /// `author/slug[:variant]`, lexicographic.
    Model,
}

/// Steps `from` `steps` positions around [`SORT_CYCLE`], wrapping.
///
/// Index arithmetic is denied by lint, so the cycle is walked by
/// `nth` over an endlessly-chained iterator.
fn cycle_step(from: SortColumn, steps: usize) -> SortColumn {
    let offset = SORT_CYCLE.iter().position(|c| *c == from).unwrap_or(0) + steps;
    SORT_CYCLE
        .iter()
        .cycle()
        .nth(offset)
        .copied()
        .unwrap_or_default()
}

impl SortColumn {
    /// The next column in the `h`/`l` cycle.
    #[must_use]
    pub fn next(self) -> Self {
        cycle_step(self, 1)
    }

    /// The previous column in the `h`/`l` cycle.
    #[must_use]
    pub fn prev(self) -> Self {
        cycle_step(self, SORT_CYCLE.len() - 1)
    }

    /// The column header the table shows.
    #[must_use]
    pub fn header(self) -> &'static str {
        match self {
            Self::Proj => "PROJ",
            Self::In => "IN/M",
            Self::Out => "OUT/M",
            Self::Cache => "CACHE/M",
            Self::Eci => "ECI",
            Self::Model => "MODEL",
        }
    }
}

/// The model-market picker's per-open state.
#[derive(Debug, Clone)]
pub struct MarketUiState {
    /// The highlighted row's index into the *filtered, sorted*
    /// projection — not into `MarketDataState.rows`.
    pub selection: usize,
    /// The filter text (`f`/`i` focuses the bar; typing narrows rows).
    pub filter: String,
    /// Whether the filter bar owns typing. While this is set the input
    /// hook claims every printable character, so `h`/`l`/`s`/`f`
    /// type into the filter instead of firing their table actions —
    /// the table actions are hook cases, not route rows, and the hook
    /// decides by focus.
    pub filter_focused: bool,
    /// The sort key; `h`/`l` cycle it, `s` flips direction.
    pub sort_col: SortColumn,
    /// The sort direction.
    pub sort_desc: bool,
    /// Whether the help card is showing (`?` toggles).
    pub help_visible: bool,
    /// Render-measured result-row count for paging; falls back to
    /// [`RESULTS_VIEWPORT_FALLBACK`] before the first frame.
    pub results_viewport: usize,
    /// Monotonic sequence the UI stamps on every fetch request so the
    /// endpoint actor can drop stale debounced fetches.
    pub fetch_seq: u64,
}

impl Default for MarketUiState {
    fn default() -> Self {
        Self {
            selection: 0,
            filter: String::new(),
            filter_focused: false,
            sort_col: SortColumn::default(),
            sort_desc: false,
            help_visible: false,
            results_viewport: RESULTS_VIEWPORT_FALLBACK,
            fetch_seq: 0,
        }
    }
}

impl MarketUiState {
    /// Resets to the opener's defaults: table focused, filter empty,
    /// PROJ ascending, help hidden.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The scope the popup's in-popup keys live in.
///
/// Dynamic and input-capturing: the scope owns the popup's keys and the
/// filter's typing carve-out, so the kernel holds no picker variant.
#[must_use]
pub fn market_ui_scope() -> SliceScopeId {
    SliceScopeId::new("model-market", "picker")
}

/// The UI cell's slot key, registered by the cell catalog before any
/// slice activates.
#[must_use]
pub fn market_ui_slot() -> SlotKey {
    SlotKey::builtin("model-market", "picker")
}
