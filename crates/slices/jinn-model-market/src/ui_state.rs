//! Pure helpers over the two market cells: the operations route actions
//! and the view need, kept out of the route closures so tests can drive
//! them directly.
//!
//! Everything here is sync-side: these functions read
//! [`MarketDataState`] and write [`MarketUiState`], never the reverse.
//! The actors' writes live in the actor modules (T2/T3).

use fuzzy_matcher::FuzzyMatcher;
use jinn_model_market_msg::MarketDataState;
use jinn_model_market_msg::MarketRow;
use jinn_model_market_msg::MarketUiState;
use jinn_model_market_msg::SortColumn;
use std::ops::Range;

/// The projected cost of one turn under the given mix, in cents.
///
/// `proj = I·(hit·cache_read_or_prompt + (1−hit)·prompt) + O·completion`
/// — locked decision F prices cache reads at the prompt price when the
/// listing publishes no cache price, so the projection is always fully
/// costed.
#[must_use]
pub fn projected_cents(row: &MarketRow, mix: (u32, f64, u32)) -> f64 {
    let (input_tokens, cache_hit, output_tokens) = mix;
    let cache_read = row.cache_read_price.unwrap_or(row.prompt_price);
    let input =
        f64::from(input_tokens) * (cache_hit * cache_read + (1.0 - cache_hit) * row.prompt_price);
    let output = f64::from(output_tokens) * row.completion_price;
    (input + output) * 100.0
}

/// The sort key for one row under the given mix.
///
/// `None` sorts after every value in either direction — an unmatched ECI
/// or an unpriced cache must not jump to the top when the user flips to
/// descending.
#[must_use]
pub fn sort_key(row: &MarketRow, col: SortColumn, mix: (u32, f64, u32)) -> Option<f64> {
    match col {
        SortColumn::Proj => Some(projected_cents(row, mix)),
        SortColumn::In => Some(row.prompt_price),
        SortColumn::Out => Some(row.completion_price),
        SortColumn::Cache => row.cache_read_price,
        SortColumn::Eci => row.eci,
        SortColumn::Model => None,
    }
}

/// The row's display name the filter matches against:
/// `author/slug[:variant]`.
#[must_use]
pub fn row_name(row: &MarketRow) -> String {
    match &row.variant {
        Some(variant) => format!("{}/{}:{variant}", row.author, row.slug),
        None => format!("{}/{}", row.author, row.slug),
    }
}

/// The visible projection: rows filtered by the fuzzy filter, then
/// stably sorted by the selected column under the active mix.
///
/// The filter runs over `author/slug[:variant]` with
/// `SkimMatcherV2`; an empty filter keeps everything. The sort is
/// stable so equal-cost rows keep their fetch order.
#[must_use]
pub fn visible_rows(data: &MarketDataState, ui: &MarketUiState) -> Vec<MarketRow> {
    let matcher = fuzzy_matcher::skim::SkimMatcherV2::default();
    let mix = ui.active_mix();
    let mut rows: Vec<MarketRow> = data
        .rows
        .iter()
        .filter(|row| {
            ui.filter.is_empty() || matcher.fuzzy_match(&row_name(row), &ui.filter).is_some()
        })
        .cloned()
        .collect();
    rows.sort_by(|a, b| compare_rows(a, b, ui, mix));
    rows
}

/// The fuzzy match offsets of `ui.filter` against one row's display
/// name — the byte indices the filter highlighted, as the
/// `[Range<usize>]` slices the selection widget's highlighter takes.
///
/// `None` means "no filter active" (nothing to highlight); an active
/// filter that misses a row cannot reach here, because the projection
/// has already dropped the row.
#[must_use]
pub fn match_ranges(row: &MarketRow, ui: &MarketUiState) -> Option<Vec<Range<usize>>> {
    if ui.filter.is_empty() {
        return None;
    }
    let matcher = fuzzy_matcher::skim::SkimMatcherV2::default();
    let name = row_name(row);
    let (_, indices) = matcher.fuzzy_indices(&name, &ui.filter)?;
    Some(indices.into_iter().map(|i| i..i + 1).collect())
}

/// Compares two rows under the UI's sort column and direction.
///
/// `None` keys sort after values regardless of direction, so a flip
/// never surfaces the unmatched rows first.
fn compare_rows(
    a: &MarketRow,
    b: &MarketRow,
    ui: &MarketUiState,
    mix: (u32, f64, u32),
) -> std::cmp::Ordering {
    match (sort_key(a, ui.sort_col, mix), sort_key(b, ui.sort_col, mix)) {
        (Some(av), Some(bv)) => {
            let ord = av.partial_cmp(&bv).unwrap_or(std::cmp::Ordering::Equal);
            if ui.sort_desc { ord.reverse() } else { ord }
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => row_name(a).cmp(&row_name(b)),
    }
}

/// The highlighted row in the visible projection, if any.
#[must_use]
pub fn highlighted_row(data: &MarketDataState, ui: &MarketUiState) -> Option<MarketRow> {
    let rows = visible_rows(data, ui);
    rows.get(ui.selection).cloned()
}

/// Moves the highlight up one row, clamped at the top.
pub fn move_up(ui: &mut MarketUiState) {
    ui.selection = ui.selection.saturating_sub(1);
}

/// Moves the highlight down one row, clamped at the projection's end.
pub fn move_down(ui: &mut MarketUiState, visible_count: usize) {
    if visible_count == 0 {
        ui.selection = 0;
        return;
    }
    ui.selection = (ui.selection + 1).min(visible_count - 1);
}

/// Pages the highlight up by the measured viewport.
pub fn page_up(ui: &mut MarketUiState) {
    ui.selection = ui.selection.saturating_sub(ui.results_viewport);
}

/// Pages the highlight down by the measured viewport.
pub fn page_down(ui: &mut MarketUiState, visible_count: usize) {
    if visible_count == 0 {
        ui.selection = 0;
        return;
    }
    ui.selection = (ui.selection + ui.results_viewport).min(visible_count - 1);
}

#[cfg(test)]
mod projected_cost_tests {
    use super::*;

    /// A `MarketRow` priced for the projection tests: everything else
    /// defaults away (empty ids, no variant, no ECI).
    fn priced_row(prompt: f64, completion: f64, cache: Option<f64>) -> MarketRow {
        MarketRow {
            id: "openrouter/a/model".to_owned(),
            author: "a".to_owned(),
            slug: "model".to_owned(),
            variant: None,
            prompt_price: prompt,
            completion_price: completion,
            cache_read_price: cache,
            has_overrides: false,
            context_length: None,
            eci: None,
            eci_ci: None,
            is_free: false,
        }
    }

    #[rstest::rstest]
    fn projected_cents_prices_cache_reads_at_the_cache_price() {
        // Given a row publishing a cache-read price.
        let row = priced_row(1e-6, 2e-6, Some(1e-7));

        // When projecting under the first preset (80k in, 99% cache
        // hit, 1.5k out).
        let cents = projected_cents(&row, (80_000, 0.99, 1_500));

        // Then the projection is
        // 80k·(0.99·cache + 0.01·prompt) + 1.5k·completion, ×100 cents.
        let expected = (80_000.0 * (0.99 * 1e-7 + 0.01 * 1e-6) + 1_500.0 * 2e-6) * 100.0;
        assert!((cents - expected).abs() < 1e-9, "{cents} vs {expected}");
    }

    #[rstest::rstest]
    fn projected_cents_falls_back_to_the_prompt_price_without_a_cache_price() {
        // Given a row with NO cache-read price (locked decision F).
        let row = priced_row(1e-6, 2e-6, None);

        // When projecting under the same mix.
        let cents = projected_cents(&row, (80_000, 0.99, 1_500));

        // Then the cache-hit share prices at the PROMPT price: the
        // projection is always fully costed.
        let expected = (80_000.0 * (0.99 * 1e-6 + 0.01 * 1e-6) + 1_500.0 * 2e-6) * 100.0;
        assert!((cents - expected).abs() < 1e-9, "{cents} vs {expected}");
    }

    #[rstest::rstest]
    fn projected_cents_scales_with_the_input_side_of_the_mix() {
        // Given one priced row.
        let row = priced_row(1e-6, 2e-6, Some(1e-7));

        // When projecting under 20k input versus 150k input (same hit
        // and output).
        let small = projected_cents(&row, (20_000, 0.99, 1_500));
        let large = projected_cents(&row, (150_000, 0.99, 1_500));

        // Then the larger input mix projects proportionally more.
        assert!(large > small);
    }
}
