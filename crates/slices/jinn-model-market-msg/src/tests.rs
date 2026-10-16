//! The contract crate's own tests: the wire invariants T2/T3 build on.

#![expect(
    clippy::expect_used,
    reason = "test module, a failed wire invariant is a test failure"
)]

use super::*;
use crate::epoch::EciScore;

// ── Cell payload behavior ────────────────────────────────────────────

#[rstest::rstest]
fn ui_state_reset_restores_the_opener_defaults() {
    // Given UI state moved away from every default, detail mode included.
    let mut state = MarketUiState {
        selection: 7,
        filter: "sonnet".to_owned(),
        filter_focused: true,
        sort_col: SortColumn::Eci,
        sort_desc: true,
        help_visible: true,
        results_viewport: 42,
        fetch_seq: 9,
        detail_model: Some("openrouter/a/model".to_owned()),
        detail_selection: 3,
        detail_sort_col: DetailSort::Tp,
        detail_sort_desc: true,
    };

    // When the opener resets it.
    state.reset();

    // Then every display field is back to the defaults the contract
    // names: table mode, table focused, filter empty, PROJ ascending.
    assert_eq!(state.selection, 0);
    assert!(state.filter.is_empty());
    assert!(!state.filter_focused);
    assert_eq!(state.sort_col, SortColumn::Proj);
    assert!(!state.sort_desc);
    assert!(!state.help_visible);
    // And the measurement fields reset too: the viewport falls back to
    // its constant until the first frame re-measures it, and the fetch
    // sequence restarts.
    assert_eq!(state.results_viewport, RESULTS_VIEWPORT_FALLBACK);
    assert_eq!(state.fetch_seq, 0);
    // And the detail-mode fields reset to table mode: no pin, the
    // detail sort on its PROJ-ascending default.
    assert_eq!(state.detail_model, None);
    assert_eq!(state.detail_selection, 0);
    assert_eq!(state.detail_sort_col, DetailSort::Proj);
    assert!(!state.detail_sort_desc);
}

#[rstest::rstest]
fn enter_detail_pins_the_model_and_resets_the_detail_side_only() {
    // Given UI state with table-side state the round trip must preserve.
    let mut state = MarketUiState {
        filter: "son".to_owned(),
        filter_focused: true,
        sort_col: SortColumn::Eci,
        sort_desc: true,
        selection: 2,
        detail_selection: 5,
        detail_sort_col: DetailSort::Tp,
        detail_sort_desc: true,
        ..MarketUiState::default()
    };

    // When detail mode is entered for a model.
    state.enter_detail("openrouter/a/model".to_owned());

    // Then the mode is pinned, and the detail-side state is on its
    // defaults.
    assert_eq!(state.detail_model, Some("openrouter/a/model".to_owned()));
    assert_eq!(state.detail_selection, 0);
    assert_eq!(state.detail_sort_col, DetailSort::Proj);
    assert!(!state.detail_sort_desc);
    // And the table side is untouched.
    assert_eq!(state.filter, "son");
    assert!(state.filter_focused);
    assert_eq!(state.sort_col, SortColumn::Eci);
    assert!(state.sort_desc);
    assert_eq!(state.selection, 2);
}

#[rstest::rstest]
fn leave_detail_clears_the_pin_and_keeps_the_table_state() {
    // Given detail mode holding table-side state.
    let mut state = MarketUiState {
        detail_model: Some("openrouter/a/model".to_owned()),
        detail_selection: 4,
        detail_sort_desc: true,
        filter: "son".to_owned(),
        sort_col: SortColumn::In,
        selection: 1,
        ..MarketUiState::default()
    };

    // When detail mode is left.
    state.leave_detail();

    // Then the pin is gone: table mode again.
    assert_eq!(state.detail_model, None);
    // And the table side is untouched.
    assert_eq!(state.filter, "son");
    assert_eq!(state.sort_col, SortColumn::In);
    assert_eq!(state.selection, 1);
}

#[rstest::rstest]
fn projection_mix_is_the_fixed_contract_mix() {
    // Given the projection constant the slice's contract names.

    // When read.
    let (input, hit, output) = PROJECTION_MIX;

    // Then it is the locked decision: 10M input tokens, 98% cache-hit
    // fraction, 50k output tokens.
    assert_eq!(input, 10_000_000);
    assert!((hit - 0.98).abs() < f64::EPSILON);
    assert_eq!(output, 50_000);
}

#[rstest::rstest]
#[case(SortColumn::Proj, SortColumn::In)]
#[case(SortColumn::In, SortColumn::Out)]
#[case(SortColumn::Out, SortColumn::Cache)]
#[case(SortColumn::Cache, SortColumn::Eci)]
#[case(SortColumn::Eci, SortColumn::Model)]
#[case(SortColumn::Model, SortColumn::Proj)]
fn sort_cycle_advances_left_to_right_wrapping(#[case] from: SortColumn, #[case] to: SortColumn) {
    assert_eq!(from.next(), to);
    // And the cycle is symmetric.
    assert_eq!(to.prev(), from);
}

#[rstest::rstest]
#[case(DetailSort::Endpoint, DetailSort::In)]
#[case(DetailSort::In, DetailSort::Out)]
#[case(DetailSort::Out, DetailSort::Cache)]
#[case(DetailSort::Cache, DetailSort::Proj)]
#[case(DetailSort::Proj, DetailSort::Lat)]
#[case(DetailSort::Lat, DetailSort::Tp)]
#[case(DetailSort::Tp, DetailSort::Up)]
#[case(DetailSort::Up, DetailSort::Endpoint)]
fn detail_sort_cycle_advances_in_header_order_wrapping(
    #[case] from: DetailSort,
    #[case] to: DetailSort,
) {
    assert_eq!(from.next(), to);
    // And the cycle is symmetric.
    assert_eq!(to.prev(), from);
}

#[rstest::rstest]
fn detail_sort_headers_name_the_eight_detail_columns() {
    // Given the eight detail sorts in cycle order.
    let headers: Vec<&str> = DETAIL_SORT_CYCLE.iter().map(|col| col.header()).collect();

    // Then each names its detail column, in header order.
    assert_eq!(
        headers,
        vec![
            "ENDPOINT", "IN /M", "OUT /M", "CACHE /M", "PROJ", "LAT p50", "TP p50", "UP 1d"
        ]
    );
}

#[rstest::rstest]
fn eci_index_lookups_key_on_the_display_name() {
    // Given an index with one model version.
    let mut index = EciIndex::default();
    index.scores.insert(
        "Claude Sonnet 4.5".to_owned(),
        EciScore {
            eci: 42.0,
            ci_low: 40.0,
            ci_high: 44.0,
        },
    );

    // When looking the name up.
    let score = index.get("Claude Sonnet 4.5");

    // Then the score and its interval come back.
    assert_eq!(
        score,
        Some(&EciScore {
            eci: 42.0,
            ci_low: 40.0,
            ci_high: 44.0,
        })
    );
    // And a missing name resolves to nothing (the `—` + `#` path).
    assert!(index.get("Unmatched Model").is_none());
    // And the census reflects the single entry.
    assert_eq!(index.len(), 1);
    assert!(!index.is_empty());
}

// ── MarketRow round-trip (the disk-cache payload) ────────────────────

#[rstest::rstest]
fn market_row_roundtrips_through_json() {
    // Given a row with every optional field set.
    let row = MarketRow {
        id: "openrouter/anthropic/claude-sonnet-4.5:thinking".to_owned(),
        author: "anthropic".to_owned(),
        slug: "claude-sonnet-4.5".to_owned(),
        variant: Some("thinking".to_owned()),
        prompt_price: 3e-6,
        completion_price: 1.5e-5,
        cache_read_price: Some(3e-7),
        has_overrides: true,
        context_length: Some(200_000),
        eci: Some(42.0),
        eci_ci: Some((40.0, 44.0)),
        is_free: false,
    };

    // When serializing and deserializing.
    let json = serde_json::to_string(&row).expect("row serializes");
    let back: MarketRow = serde_json::from_str(&json).expect("row deserializes");

    // Then the roundtrip is lossless.
    assert_eq!(back, row);
}

#[rstest::rstest]
fn market_data_state_defaults_to_a_clean_table() {
    // Given a fresh data cell payload.
    let state = MarketDataState::default();

    // Then nothing has loaded: empty rows, no index, no stamps, no
    // in-flight flags, seed source, no error.
    assert!(state.rows.is_empty());
    assert!(state.detail.is_none());
    assert!(state.eci.is_none());
    assert!(state.models_fetched_at.is_none());
    assert!(state.epoch_fetched_at.is_none());
    assert!(state.endpoints_fetched_at.is_none());
    assert!(!state.models_loading);
    assert!(!state.endpoints_loading);
    assert!(!state.epoch_loading);
    assert_eq!(state.epoch_source, EpochSource::Seed);
    assert!(state.last_error.is_none());
}
