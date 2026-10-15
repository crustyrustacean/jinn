//! The model-market popup's overlay rendering: the centered full-table
//! geometry, the table itself, the status line, and the footer.
//!
//! The render pass reads the cells only — no IO, no actor pokes, no
//! messages published from render. PROJ is computed here from the live
//! mix so `m` re-sorts the table on the next frame with no refetch.

use jinn_model_market_msg::MarketDataState;
use jinn_model_market_msg::MarketUiState;
use jinn_model_market_msg::SortColumn;
use jinn_slices::RenderFacts;
use jinn_theme::Theme;
use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Cell;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;

use crate::format;
use crate::ui_state;

/// The popup rect: a centered table window, ~90% of the frame with
/// minimums so a small terminal still draws a usable table.
#[must_use]
pub fn market_overlay_rect(area: &Rect) -> Option<Rect> {
    if area.width < 40 || area.height < 12 {
        return None;
    }
    let width = ((area.width * 9) / 10).max(40);
    let height = ((area.height * 8) / 10).max(12);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Some(Rect::new(x, y, width, height))
}

/// Draws the popup for one frame: table, status line, footer.
pub fn render_market_popup(frame: &mut Frame<'_>, area: Rect, facts: &RenderFacts) {
    let Some(ui_cell) = facts
        .slices
        .reader::<MarketUiState>(&crate::market_ui_slot())
    else {
        return;
    };
    let Some(data_cell) = facts
        .slices
        .reader::<MarketDataState>(&crate::market_data_slot())
    else {
        return;
    };

    // Publish the measured result-row count for the paging keys before
    // reading (the project picker's measurement pattern).
    let measured = results_viewport(area);
    ui_cell.update(|state: &mut MarketUiState| state.results_viewport = measured);

    let theme = facts.theme.clone();
    let visible = {
        let data = data_cell.read();
        let ui = ui_cell.read();
        crate::ui_state::visible_rows(&data, &ui)
    };

    let ui = ui_cell.read();
    let data = data_cell.read();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " Model Market ",
            Style::default()
                .fg(theme.popup_title)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme.muted_text));

    let header = header_row(&ui);
    let rows: Vec<Row<'_>> = visible
        .iter()
        .enumerate()
        .map(|(idx, row)| table_row(row, idx == ui.selection, &theme))
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(24),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(6),
        ],
    )
    .header(header)
    .block(block)
    .column_spacing(1);

    // The highlighted row stays inside the visible window the paging
    // keys clamp against.
    let mut table_state = TableState::default().with_selected(Some(ui.selection));
    frame.render_stateful_widget(table, area, &mut table_state);

    render_status_line(frame, area, &ui, &data, &theme);
}

/// The table header row, with the active sort column marked.
fn header_row(ui: &MarketUiState) -> Row<'static> {
    let marker = if ui.sort_desc {
        " \u{2193}"
    } else {
        " \u{2191}"
    };
    let cells: Vec<Cell<'static>> = sort_column_cycle()
        .iter()
        .map(|col| {
            let name = col.header();
            let text = if *col == ui.sort_col {
                format!("{name}{marker}")
            } else {
                name.to_owned()
            };
            Cell::from(text).style(Style::default().add_modifier(Modifier::BOLD))
        })
        .collect();
    Row::new(cells)
}

/// The sortable columns in table order (the `h`/`l` cycle order).
fn sort_column_cycle() -> &'static [SortColumn] {
    &[
        SortColumn::Proj,
        SortColumn::In,
        SortColumn::Out,
        SortColumn::Cache,
        SortColumn::Eci,
        SortColumn::Model,
    ]
}

/// One table row: the contract's columns for one listing.
fn table_row<'a>(
    row: &'a jinn_model_market_msg::MarketRow,
    selected: bool,
    theme: &Theme,
) -> Row<'a> {
    let ui = MarketUiState::default();
    let mix = ui.active_mix();
    let cells = [
        Cell::from(model_column(row)),
        Cell::from(format::price_per_million(row.prompt_price)),
        Cell::from(format::price_per_million(row.completion_price)),
        Cell::from(format::cache_column(row.cache_read_price)),
        Cell::from(format::cents(ui_state::projected_cents(row, mix))),
        Cell::from(format::eci_column(row.eci)),
        Cell::from(flags_column(row)),
    ];
    if selected {
        Row::new(cells).style(
            Style::default()
                .fg(theme.primary_text)
                .bg(theme.picker_selected_bg),
        )
    } else {
        Row::new(cells)
    }
}

/// The MODEL column: `author/slug` plus a dimmed `:variant`.
fn model_column(row: &jinn_model_market_msg::MarketRow) -> String {
    match &row.variant {
        Some(variant) => format!("{}/{}:{variant}", row.author, row.slug),
        None => format!("{}/{}", row.author, row.slug),
    }
}

/// The FLAGS column: `†` pricing overrides, `f` `:free`, `#` no ECI.
fn flags_column(row: &jinn_model_market_msg::MarketRow) -> String {
    let mut flags = String::new();
    if row.has_overrides {
        flags.push('\u{2020}');
    }
    if row.is_free {
        flags.push('f');
    }
    if row.eci.is_none() {
        flags.push('#');
    }
    flags
}

/// The status line, drawn on the popup's bottom-inner row: the three
/// "updated X ago" stamps (rendered from the stamps, no network), the
/// active mix, the epoch source, and the degraded-mode/error line.
fn render_status_line(
    frame: &mut Frame<'_>,
    area: Rect,
    ui: &MarketUiState,
    data: &MarketDataState,
    theme: &Theme,
) {
    let status_y = area.y + area.height.saturating_sub(2);
    let status_area = Rect::new(area.x + 1, status_y, area.width.saturating_sub(2), 1);
    let muted = Style::default().fg(theme.muted_text);

    let mut spans = Vec::new();
    if let Some(stamp) = data.models_fetched_at {
        spans.push(Span::styled(
            format!("models updated {} \u{b7} ", format::age_stamp(stamp)),
            muted,
        ));
    }
    if let Some(stamp) = data.epoch_fetched_at {
        spans.push(Span::styled(
            format!("epoch {} \u{b7} ", format::age_stamp(stamp)),
            muted,
        ));
    }
    if let Some(stamp) = data.endpoints_fetched_at {
        spans.push(Span::styled(
            format!("endpoints {} \u{b7} ", format::age_stamp(stamp)),
            muted,
        ));
    }
    spans.push(Span::styled(format::mix_summary(ui), muted));
    spans.push(Span::styled(
        format!(" \u{b7} epoch: {}", data.epoch_source.label()),
        muted,
    ));
    if data.models_loading || data.endpoints_loading || data.epoch_loading {
        spans.push(Span::styled(" \u{b7} loading\u{2026}", muted));
    }
    if let Some(error) = &data.last_error {
        spans.push(Span::styled(
            format!(" \u{b7} {error}"),
            Style::default().fg(theme.accent_action),
        ));
    }
    frame.render_widget(Line::from(spans), status_area);
}

/// The popup's result-row count for a frame of the given size: the popup
/// height less both borders, the header, and the status line. At least
/// 1, so paging never windows against zero.
#[must_use]
pub fn results_viewport(popup: Rect) -> usize {
    let inner = popup.height.saturating_sub(2);
    inner.saturating_sub(2).max(1) as usize
}

/// Helper for `Filter` mode display in the footer — the filter bar
/// renders inside the block title area when focused.
#[must_use]
pub fn filter_title(filter: &str, focused: bool) -> String {
    if focused {
        format!(" filter: {filter}\u{2588} ")
    } else {
        String::new()
    }
}
