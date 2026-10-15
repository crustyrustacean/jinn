//! Display formatting shared by the view and the tests: price columns,
//! the "updated X ago" stamps, and the status-line fragments.

use jiff::Timestamp;
use jinn_model_market_msg::MarketUiState;

/// Formats a USD/token price as a `$X/M` column value, trimmed of
/// meaningless precision (e.g. `$3.00/M`, `$0.30/M`, `$1.25/M`).
#[must_use]
pub fn price_per_million(usd_per_token: f64) -> String {
    let per_million = usd_per_token * 1_000_000.0;
    if per_million >= 100.0 {
        format!("${per_million:.0}/M")
    } else if per_million >= 10.0 {
        format!("${per_million:.1}/M")
    } else {
        format!("${per_million:.2}/M")
    }
}

/// Formats a USD/token price as `—` when it is unpublished (locked
/// decision F: the row is still fully costed at the prompt price).
#[must_use]
pub fn cache_column(usd_per_token: Option<f64>) -> String {
    usd_per_token.map_or_else(|| "\u{2014}".to_owned(), price_per_million)
}

/// Formats a projected cost in cents with two decimals (e.g. `1.24¢`).
#[must_use]
pub fn cents(cents: f64) -> String {
    format!("{cents:.2}\u{a2}")
}

/// Formats an ECI value, `—` when unmatched.
#[must_use]
pub fn eci_column(eci: Option<f64>) -> String {
    eci.map_or_else(|| "\u{2014}".to_owned(), |v| format!("{v:.1}"))
}

/// The elapsed time since `stamp` as a compact human string
/// (`42s ago`, `7m ago`, `3h ago`), the models picker's form.
#[must_use]
pub fn age_stamp(fetched_at: Timestamp) -> String {
    let elapsed = Timestamp::now() - fetched_at;
    let secs = elapsed.total(jiff::Unit::Second).unwrap_or(0.0).max(0.0) as u64;
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 60 * 60 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}

/// The status line's mix fragment for the active preset, e.g.
/// `mix: 80k in @99% cache \u{b7} 1.5k out`.
#[must_use]
pub fn mix_summary(ui: &MarketUiState) -> String {
    let (input, hit, output) = ui.active_mix();
    format!(
        "mix: {}k in @{}% cache \u{b7} {}k out",
        input / 1_000,
        (hit * 100.0).round() as u32,
        output / 1_000
    )
}
