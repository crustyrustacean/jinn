//! Tests for the detail pane's render behaviors: the endpoint rows that
//! appear under the table for the highlighted model, the auth-gated and
//! unpublished fields that render `—`, the fraction uptime that renders
//! as a percentage, the pricing-override rows, the stale-detail guard,
//! and the absent-detail no-op.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use jinn_model_market_msg::MarketDataState;
use jinn_model_market_msg::MarketDetail;
use jinn_model_market_msg::MarketEndpointRow;
use jinn_model_market_msg::MarketRow;
use jinn_model_market_msg::MarketUiState;
use jinn_slices::Slices;
use jinn_slices::cell::TypedCell;
use ratatui::buffer::Buffer;
use ratatui::buffer::Cell;

/// A registered pair of cells over a fresh registry, exactly as the
/// composition catalog seeds them — the render pass resolves readers
/// through the facts context.
struct Harness {
    slices: Slices,
    ui: TypedCell<MarketUiState>,
    data: TypedCell<MarketDataState>,
}

impl Harness {
    fn new() -> Self {
        let slices = Slices::new();
        let ui = slices
            .register(crate::market_ui_slot(), MarketUiState::default())
            .expect("fresh registry has no UI slot yet");
        let data = slices
            .register(crate::market_data_slot(), MarketDataState::default())
            .expect("fresh registry has no data slot yet");
        Self { slices, ui, data }
    }

    /// Renders one frame at the given popup rect against a test backend
    /// and returns the buffer.
    fn render(&self, popup: ratatui::layout::Rect, width: u16, height: u16) -> Buffer {
        let theme = jinn_theme::default_theme();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                .expect("test terminal");
        terminal
            .draw(|frame| {
                let facts = jinn_slices::RenderFacts::new(theme.clone(), &self.slices);
                crate::view::render_market_popup(frame, popup, &facts);
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }
}

/// One listing the pane can follow.
fn listed_row(id: &str) -> MarketRow {
    let (author, slug) = id
        .strip_prefix("openrouter/")
        .and_then(|rest| rest.split_once('/'))
        .unwrap_or(("a", "model"));
    MarketRow {
        id: id.to_owned(),
        author: author.to_owned(),
        slug: slug.to_owned(),
        variant: None,
        prompt_price: 1e-6,
        completion_price: 2e-6,
        cache_read_price: Some(1e-7),
        has_overrides: false,
        context_length: Some(128_000),
        eci: None,
        eci_ci: None,
        is_free: false,
    }
}

/// One endpoint with everything published: the populated-pane shape.
fn full_endpoint() -> MarketEndpointRow {
    MarketEndpointRow {
        tag: "azure/us-east".to_owned(),
        provider_name: "Azure".to_owned(),
        prompt_price: 1.2e-6,
        completion_price: 4.5e-6,
        cache_read_price: Some(0.9e-7),
        latency_p50_ms: Some(320.0),
        throughput_p50: Some(88.0),
        uptime_1d: Some(0.992),
        quantization: None,
        overrides: Vec::new(),
    }
}

/// The detail payload the cell carries once the actor's fetch lands.
fn detail_for(id: &str, endpoints: Vec<MarketEndpointRow>) -> MarketDetail {
    MarketDetail {
        model_id: id.to_owned(),
        endpoints,
    }
}

/// The buffer's full text, cells joined.
fn buffer_text(buffer: &Buffer) -> String {
    buffer.content.iter().map(Cell::symbol).collect()
}

/// A rendered row's text, cells joined across the popup's width.
fn row_text(buffer: &Buffer, y: u16, width: u16) -> String {
    (0..width)
        .filter_map(|x| buffer.cell((x, y)).map(Cell::symbol))
        .collect()
}

#[rstest::rstest]
fn pane_renders_the_highlighted_model_endpoint_rows() {
    // Given a table whose highlighted row's endpoints have landed in the
    // data cell.
    let harness = Harness::new();
    harness.ui.update(|s| s.selection = 0);
    harness.data.update(|d| {
        d.rows = vec![listed_row("openrouter/a/model")];
        d.detail = Some(detail_for("openrouter/a/model", vec![full_endpoint()]));
    });
    let popup = ratatui::layout::Rect::new(5, 2, 130, 16);

    // When the popup renders.
    let buffer = harness.render(popup, 140, 100);

    // Then the pane draws under the table: the model's header, the
    // endpoint's name and tag, and every published field.
    let text = buffer_text(&buffer);
    assert!(
        text.contains("openrouter/a/model endpoints"),
        "the pane header names the highlighted model, got: {text:?}"
    );
    assert!(
        text.contains("Azure (azure/us-east)"),
        "the endpoint renders name and tag, got: {text:?}"
    );
    assert!(
        text.contains("IN $1.2000/M") && text.contains("OUT $4.5000/M"),
        "the endpoint's prices render, got: {text:?}"
    );
    assert!(
        text.contains("LAT p50 320ms") && text.contains("TP p50 88 tok/s"),
        "the endpoint's percentiles render, got: {text:?}"
    );
}

#[rstest::rstest]
fn pane_renders_null_percentiles_as_em_dashes() {
    // Given an endpoint whose LAT/TP arrived null (no API key — the
    // percentiles are auth-gated).
    let harness = Harness::new();
    harness.ui.update(|s| s.selection = 0);
    let mut endpoint = full_endpoint();
    endpoint.latency_p50_ms = None;
    endpoint.throughput_p50 = None;
    harness.data.update(|d| {
        d.rows = vec![listed_row("openrouter/a/model")];
        d.detail = Some(detail_for("openrouter/a/model", vec![endpoint]));
    });
    let popup = ratatui::layout::Rect::new(5, 2, 130, 16);

    // When the popup renders.
    let buffer = harness.render(popup, 140, 100);

    // Then both percentile fields read the em-dash, and no made-up
    // number stands in for the missing data.
    let text = buffer_text(&buffer);
    assert!(
        text.contains("LAT p50 \u{2014}") && text.contains("TP p50 \u{2014}"),
        "null percentiles render em-dashes, got: {text:?}"
    );
    assert!(
        !text.contains("320ms") && !text.contains("88 tok/s"),
        "no stale percentile values survive, got: {text:?}"
    );
}

#[rstest::rstest]
fn pane_renders_null_cache_as_em_dash() {
    // Given an endpoint publishing no cache-read price (decision F).
    let harness = Harness::new();
    harness.ui.update(|s| s.selection = 0);
    let mut endpoint = full_endpoint();
    endpoint.cache_read_price = None;
    harness.data.update(|d| {
        d.rows = vec![listed_row("openrouter/a/model")];
        d.detail = Some(detail_for("openrouter/a/model", vec![endpoint]));
    });
    let popup = ratatui::layout::Rect::new(5, 2, 130, 16);

    // When the popup renders.
    let buffer = harness.render(popup, 140, 100);

    // Then the CACHE field reads the em-dash.
    let text = buffer_text(&buffer);
    assert!(
        text.contains("CACHE \u{2014}"),
        "a null cache price renders the em-dash, got: {text:?}"
    );
}

#[rstest::rstest]
fn pane_renders_uptime_fraction_as_a_percentage() {
    // Given an endpoint whose uptime arrived as the fraction 0.992 — the
    // actor's locked conversion from the wire's percentage.
    let harness = Harness::new();
    harness.ui.update(|s| s.selection = 0);
    harness.data.update(|d| {
        d.rows = vec![listed_row("openrouter/a/model")];
        d.detail = Some(detail_for("openrouter/a/model", vec![full_endpoint()]));
    });
    let popup = ratatui::layout::Rect::new(5, 2, 130, 16);

    // When the popup renders.
    let buffer = harness.render(popup, 140, 100);

    // Then the UP 1d field reads the ×100 percentage, not the raw
    // fraction.
    let text = buffer_text(&buffer);
    assert!(
        text.contains("UP 1d 99.2%"),
        "the fraction renders ×100 with %, got: {text:?}"
    );
    assert!(
        !text.contains("0.99"),
        "the raw fraction never reaches the screen, got: {text:?}"
    );
}

#[rstest::rstest]
fn pane_renders_the_model_eci_with_confidence_interval() {
    // Given a highlighted row joined with an ECI score and CI, its
    // endpoints fetched.
    let harness = Harness::new();
    harness.ui.update(|s| s.selection = 0);
    harness.data.update(|d| {
        let mut row = listed_row("openrouter/a/model");
        row.eci = Some(166.8);
        row.eci_ci = Some((163.5, 171.0));
        d.rows = vec![row];
        d.detail = Some(detail_for("openrouter/a/model", vec![full_endpoint()]));
    });
    let popup = ratatui::layout::Rect::new(5, 2, 130, 16);

    // When the popup renders.
    let buffer = harness.render(popup, 140, 100);

    // Then the pane states the ECI ± its bootstrap interval.
    let text = buffer_text(&buffer);
    assert!(
        text.contains("ECI 166.8 \u{b1} [163.5, 171.0]"),
        "the ECI line carries the score ± CI, got: {text:?}"
    );
}

#[rstest::rstest]
fn pane_renders_pricing_override_rows() {
    // Given an endpoint carrying a conditional pricing override above a
    // minimum-prompt-token threshold.
    let harness = Harness::new();
    harness.ui.update(|s| s.selection = 0);
    let mut endpoint = full_endpoint();
    endpoint.overrides = vec![(Some(200_000), 6e-6, 1.2e-5)];
    harness.data.update(|d| {
        d.rows = vec![listed_row("openrouter/a/model")];
        d.detail = Some(detail_for("openrouter/a/model", vec![endpoint]));
    });
    let popup = ratatui::layout::Rect::new(5, 2, 130, 16);

    // When the popup renders.
    let buffer = harness.render(popup, 140, 100);

    // Then the override renders its own row in the ">Nk ctx" shape.
    let text = buffer_text(&buffer);
    assert!(
        text.contains(">200k ctx: in $6.0000/M / out $12.0000/M / cch \u{2014}"),
        "the override row renders, got: {text:?}"
    );
}

#[rstest::rstest]
fn pane_renders_a_thresholdless_override_as_a_wildcard_row() {
    // Given an endpoint carrying a pricing override with NO minimum
    // prompt-token threshold — the shape the upstream publishes for
    // "any prompt above the endpoint's base price".
    let harness = Harness::new();
    harness.ui.update(|s| s.selection = 0);
    let mut endpoint = full_endpoint();
    endpoint.overrides = vec![(None, 6e-6, 1.2e-5)];
    harness.data.update(|d| {
        d.rows = vec![listed_row("openrouter/a/model")];
        d.detail = Some(detail_for("openrouter/a/model", vec![endpoint]));
    });
    let popup = ratatui::layout::Rect::new(5, 2, 130, 16);

    // When the popup renders.
    let buffer = harness.render(popup, 140, 100);

    // Then the override renders its own row under the `*` wildcard —
    // a thresholdless override must not be silently dropped.
    let text = buffer_text(&buffer);
    assert!(
        text.contains("*: in $6.0000/M / out $12.0000/M / cch \u{2014}"),
        "the thresholdless override renders as a wildcard row, got: {text:?}"
    );
}

#[rstest::rstest]
fn page_down_keeps_the_selected_row_above_the_detail_pane() {
    // Given a long table with a multi-endpoint detail pane visible —
    // the pane covers the table's tail rows.
    let harness = Harness::new();
    let mut endpoint = full_endpoint();
    endpoint.overrides = vec![(Some(200_000), 6e-6, 1.2e-5), (None, 5e-6, 1.0e-5)];
    let rows: Vec<MarketRow> = (0..30)
        .map(|i| listed_row(&format!("openrouter/m/model{i}")))
        .collect();
    harness.data.update(|d| {
        d.rows = rows;
        d.detail = Some(detail_for("openrouter/m/model0", vec![endpoint]));
    });
    harness.ui.update(|s| s.selection = 0);
    let popup = ratatui::layout::Rect::new(5, 2, 130, 40);

    // When a frame renders (publishing the pane-aware measurement) and
    // the selection then pages down, with the detail following it (the
    // debounce caught up) so the pane stays visible.
    harness.render(popup, 140, 100);
    let viewport = harness.ui.read().results_viewport;
    assert!(viewport > 0, "a frame measured the paging window");
    let rows = harness.data.read().rows.len();
    let landed = viewport.min(rows - 1).saturating_sub(1);
    harness.ui.update(|s| s.selection = landed);
    harness.data.update(|d| {
        if let Some(detail) = &mut d.detail {
            detail.model_id = format!("openrouter/m/model{landed}");
        }
    });

    // Then the selected row renders ABOVE the pane's top edge — the
    // paging window never lands the highlight in the covered zone.
    let buffer = harness.render(popup, 140, 100);
    let pane_top = (popup.y..popup.y + popup.height)
        .find(|y| row_text(&buffer, *y, popup.width).contains("endpoints"))
        .expect("the pane renders");
    let selected_y = (popup.y..popup.y + popup.height)
        .find(|y| {
            row_text(&buffer, *y, popup.width).contains("m/model")
                && (popup.x..popup.x + popup.width).any(|x| {
                    buffer
                        .cell((x, *y))
                        .is_some_and(|c| c.bg == jinn_theme::default_theme().picker_selected_bg)
                })
        })
        .expect("a selected row renders");
    assert!(
        selected_y < pane_top,
        "the selection ({selected_y}) must sit above the pane ({pane_top})"
    );
}

#[rstest::rstest]
fn pane_suppresses_stale_detail_for_a_row_the_user_left() {
    // Given a data cell still holding the previous model's endpoints
    // while the highlight has moved on — the debounce has not caught up.
    let harness = Harness::new();
    harness.ui.update(|s| s.selection = 0);
    harness.data.update(|d| {
        d.rows = vec![
            listed_row("openrouter/a/model"),
            listed_row("openrouter/b/next"),
        ];
        d.detail = Some(detail_for("openrouter/a/model", vec![full_endpoint()]));
        d.rows = vec![listed_row("openrouter/b/next")];
    });
    let popup = ratatui::layout::Rect::new(5, 2, 130, 16);

    // When the popup renders.
    let buffer = harness.render(popup, 140, 100);

    // Then the stale detail does not render — no endpoint text anywhere
    // on screen.
    let text = buffer_text(&buffer);
    assert!(
        !text.contains("Azure") && !text.contains("endpoints"),
        "stale detail must not render, got: {text:?}"
    );
}

#[rstest::rstest]
fn pane_renders_nothing_when_detail_is_absent() {
    // Given a highlighted row whose detail has not landed yet (the
    // debounced fetch is still in flight).
    let harness = Harness::new();
    harness.ui.update(|s| s.selection = 0);
    harness.data.update(|d| {
        d.rows = vec![listed_row("openrouter/a/model")];
    });
    let popup = ratatui::layout::Rect::new(5, 2, 130, 16);

    // When the popup renders.
    let buffer = harness.render(popup, 140, 100);

    // Then no pane draws at all — no placeholder, no empty box, no pane
    // text anywhere.
    let text = buffer_text(&buffer);
    assert!(
        !text.contains("endpoints") && !text.contains("LAT p50"),
        "absent detail renders nothing, got: {text:?}"
    );
}

#[rstest::rstest]
fn pane_follows_the_highlight_to_the_second_row() {
    // Given two models, each with its own detail, highlight on the
    // second.
    let harness = Harness::new();
    harness.ui.update(|s| s.selection = 1);
    harness.data.update(|d| {
        d.rows = vec![
            listed_row("openrouter/a/model"),
            listed_row("openrouter/b/next"),
        ];
        let mut second = full_endpoint();
        second.provider_name = "Lambda".to_owned();
        second.tag = String::new();
        d.detail = Some(detail_for("openrouter/b/next", vec![second]));
    });
    let popup = ratatui::layout::Rect::new(5, 2, 130, 16);

    // When the popup renders.
    let buffer = harness.render(popup, 140, 100);

    // Then the pane shows the second model's endpoints, the tagless
    // endpoint rendering bare-name without empty parens.
    let text = buffer_text(&buffer);
    assert!(
        text.contains("Lambda"),
        "the highlighted model's endpoint renders, got: {text:?}"
    );
    assert!(
        !text.contains("()"),
        "an empty tag renders no empty parens, got: {text:?}"
    );
    assert!(
        !text.contains("Azure"),
        "the first model's endpoint stays off screen, got: {text:?}"
    );
}

#[rstest::rstest]
fn pane_sits_above_the_status_line_and_below_the_table() {
    // Given a highlighted row with a one-endpoint detail payload.
    let harness = Harness::new();
    harness.ui.update(|s| s.selection = 0);
    harness.data.update(|d| {
        d.rows = vec![listed_row("openrouter/a/model")];
        d.detail = Some(detail_for("openrouter/a/model", vec![full_endpoint()]));
    });
    let popup = ratatui::layout::Rect::new(5, 2, 130, 16);

    // When the popup renders.
    let buffer = harness.render(popup, 140, 100);

    // Then the pane's header sits between the table's last data row and
    // the popup's status/footer rows — the second region under the
    // table, never over the status line.
    let pane_y = (popup.y..popup.y + popup.height)
        .find(|y| row_text(&buffer, *y, popup.width).contains("openrouter/a/model endpoints"))
        .expect("the pane header renders");
    let status_y = popup.y + popup.height - 3;
    assert!(
        pane_y < status_y,
        "the pane sits above the status row ({pane_y} vs {status_y})"
    );
    // And the header row the pane shares the grid with is intact above.
    let header_y = (popup.y..pane_y)
        .find(|y| row_text(&buffer, *y, popup.width).contains("MODEL"))
        .expect("the table header renders above the pane");
    assert!(header_y < pane_y, "the table header precedes the pane");
}
