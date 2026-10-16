//! Tests for the popup view's own render behaviors: the `?` help card
//! appearing with the popup's draw pass, and the MODEL column's dimmed
//! variant suffix — the card calls the suffix a dimmed one, so the
//! table must render one.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use jinn_model_market_msg::MarketDataState;
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

/// One listing with a variant, dimmed-suffix territory.
fn variant_row() -> MarketRow {
    MarketRow {
        id: "openrouter/z/mod:free".to_owned(),
        author: "z".to_owned(),
        slug: "mod".to_owned(),
        variant: Some("free".to_owned()),
        prompt_price: 1e-6,
        completion_price: 2e-6,
        cache_read_price: None,
        has_overrides: false,
        context_length: Some(128_000),
        eci: None,
        eci_ci: None,
        is_free: true,
    }
}

/// One plain listing without a variant — table filler that keeps the
/// variant row unselected (the selection row's style overrides the
/// cell foreground, hiding the dim). Priced cheaper so the default
/// PROJ-ascending sort keeps the variant row off the selection.
fn plain_row() -> MarketRow {
    MarketRow {
        id: "openrouter/a/plain".to_owned(),
        author: "a".to_owned(),
        slug: "plain".to_owned(),
        variant: None,
        prompt_price: 1e-9,
        completion_price: 1e-9,
        cache_read_price: None,
        has_overrides: false,
        context_length: Some(128_000),
        eci: None,
        eci_ci: None,
        is_free: false,
    }
}

/// The buffer's full text, cells joined.
fn buffer_text(buffer: &Buffer) -> String {
    buffer.content.iter().map(Cell::symbol).collect()
}

/// A rendered table row's text, cells joined across the popup's width.
fn row_text(buffer: &Buffer, y: u16, width: u16) -> String {
    (0..width)
        .filter_map(|x| buffer.cell((x, y)).map(Cell::symbol))
        .collect()
}

#[rstest::rstest]
fn hidden_help_draws_no_card_pixels() {
    // Given a wired popup with help hidden (the opener default).
    let harness = Harness::new();
    let popup = ratatui::layout::Rect::new(5, 2, 80, 10);

    // When the popup renders over a tall terminal.
    let buffer = harness.render(popup, 90, 100);

    // Then no help card text is anywhere on the screen.
    let text = buffer_text(&buffer);
    assert!(
        !text.contains("Model Market help"),
        "a hidden help must not draw, got: {text:?}"
    );
}

#[rstest::rstest]
fn visible_help_draws_the_card_below_the_popup() {
    // Given a wired popup with the help toggled on.
    let harness = Harness::new();
    harness.ui.update(|s| s.help_visible = true);
    let popup = ratatui::layout::Rect::new(5, 2, 80, 10);

    // When the popup renders over a terminal with room below it.
    let buffer = harness.render(popup, 90, 100);

    // Then the card is on screen directly below the popup's bottom
    // border: its top border flush with the popup's edge, its title
    // one row inside that.
    let title_row = row_text(&buffer, popup.y + popup.height + 1, popup.width);
    assert!(
        title_row.contains("Model Market help"),
        "the card draws below the popup, got: {title_row:?}"
    );
}

#[rstest::rstest]
fn visible_help_over_a_full_height_popup_still_reaches_the_screen() {
    // Given the standard centered popup (no room below it) with the
    // help toggled on.
    let harness = Harness::new();
    harness.ui.update(|s| s.help_visible = true);
    let area = ratatui::layout::Rect::new(0, 0, 120, 40);
    let popup = crate::view::market_overlay_rect(&area).expect("frame fits the popup");

    // When the popup renders.
    let buffer = harness.render(popup, 120, 40);

    // Then the card's fallback placement still puts its text on screen.
    let text = buffer_text(&buffer);
    assert!(
        text.contains("Model Market help"),
        "the card falls back to the terminal top, got: {text:?}"
    );
}

#[rstest::rstest]
fn model_column_dims_the_variant_suffix() {
    // Given a popup with a variant listing below the selected row.
    let harness = Harness::new();
    harness.ui.update(|s| s.selection = 0);
    harness.data.update(|d| {
        d.rows = vec![plain_row(), variant_row()];
    });
    let popup = ratatui::layout::Rect::new(5, 2, 80, 10);

    // When the popup renders.
    let buffer = harness.render(popup, 90, 100);

    // Then the suffix glyphs wear the muted foreground while the base
    // name does not: the card's "dimmed :variant" is what the table
    // shows. The suffix cell is found by scanning the row's buffer
    // cells for the variant's colon — string indexes and buffer
    // positions need not agree for that.
    let theme = jinn_theme::default_theme();
    let name = "z/mod:free";
    let y = (popup.y + 2..popup.y + popup.height)
        .find(|y| row_text(&buffer, *y, popup.width).contains(name))
        .expect("the variant row renders");
    let colon_x = (0..popup.width)
        .find(|x| buffer.cell((*x, y)).is_some_and(|c| c.symbol() == ":"))
        .expect("the variant suffix renders");
    let suffix_cell = buffer.cell((colon_x, y)).expect("suffix cell exists");
    assert_eq!(
        suffix_cell.style().fg,
        Some(theme.muted_text),
        "the :variant suffix renders dimmed"
    );
    let base_cell = buffer.cell((colon_x - 1, y)).expect("base cell exists");
    assert_ne!(
        base_cell.style().fg,
        Some(theme.muted_text),
        "the author/slug base stays undimmed"
    );
}

#[rstest::rstest]
fn filter_match_on_a_suffix_glyph_highlights_over_the_dim() {
    // Given a popup filtered to the variant text itself.
    let harness = Harness::new();
    harness.ui.update(|s| s.filter = "free".to_owned());
    harness.data.update(|d| {
        d.rows = vec![variant_row()];
    });
    let popup = ratatui::layout::Rect::new(5, 2, 80, 10);

    // When the popup renders.
    let buffer = harness.render(popup, 90, 100);

    // Then the matched suffix glyphs carry the shared highlight
    // background — the dim yields to the filter's highlight, as it
    // does for the base name.
    let theme = jinn_theme::default_theme();
    let y = (popup.y + 2..popup.y + popup.height)
        .find(|y| row_text(&buffer, *y, popup.width).contains("z/mod:free"))
        .expect("the filtered row renders");
    let highlighted: String = (0..popup.width)
        .filter(|x| {
            buffer
                .cell((*x, y))
                .is_some_and(|c| c.style().bg == Some(theme.picker_highlight_bg))
        })
        .filter_map(|x| buffer.cell((x, y)).map(Cell::symbol))
        .collect();
    assert_eq!(highlighted, "free", "the suffix match highlights");
}
