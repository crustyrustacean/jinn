//! The model-market popup's overlay rendering: the centered full-table
//! geometry, the table itself, the filter bar, the status line, the
//! keybind footer, and the `?` help card the view draws below itself
//! while it is toggled on.
//!
//! The render pass reads the cells only — no IO, no actor pokes, no
//! messages published from render. PROJ is computed here from the live
//! mix so `m` re-sorts the table on the next frame with no refetch.
//!
//! The table's shape has one source of truth: [`MARKET_COLUMNS`]. The
//! header, every data row, and the layout constraints all derive from
//! it, so a header cannot sit over the wrong data column.

use std::ops::Range;

use jinn_model_market_msg::MarketDataState;
use jinn_model_market_msg::MarketUiState;
use jinn_model_market_msg::SortColumn;
use jinn_slices::RenderFacts;
use jinn_theme::Theme;
use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Color;
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
use crate::help::market_help_card;
use crate::routes::MARKET_BINDINGS;
use crate::ui_state;

#[cfg(test)]
#[path = "view_tests.rs"]
mod view_tests;

/// One table column: its header, which [`SortColumn`] keys its sort,
/// whether the sort marker rides this column, its width, and the cell
/// renderer for a listing row. A renderer receives the row, the UI
/// state (the filter's match ranges), and the theme — the MODEL column
/// needs all three to draw its variant suffix dimmed under the filter's
/// highlight.
struct MarketColumn {
    header: &'static str,
    sort: SortColumn,
    /// The active-sort arrow renders on this column. `false` for a
    /// display-only column sharing another column's sort key.
    keyed: bool,
    width: u16,
    cell: fn(&jinn_model_market_msg::MarketRow, &MarketUiState, &Theme) -> Line<'static>,
}

/// The columns in table order — the same order as the `h`/`l` sort
/// cycle, [`SORT_CYCLE`], plus the FLAGS display column. FLAGS exists
/// because `#` (no ECI) is a display fact, not a sortable key: it
/// renders beside the ECI column it annotates.
const MARKET_COLUMNS: &[MarketColumn] = &[
    MarketColumn {
        header: "PROJ \u{a2}",
        sort: SortColumn::Proj,
        keyed: true,
        width: 9,
        cell: |row, _ui, _theme| {
            let mix = MarketUiState::default().active_mix();
            Line::from(format::cents(ui_state::projected_cents(row, mix)))
        },
    },
    MarketColumn {
        header: "IN/M",
        sort: SortColumn::In,
        keyed: true,
        width: 9,
        cell: |row, _ui, _theme| Line::from(format::price_per_million(row.prompt_price)),
    },
    MarketColumn {
        header: "OUT/M",
        sort: SortColumn::Out,
        keyed: true,
        width: 9,
        cell: |row, _ui, _theme| Line::from(format::price_per_million(row.completion_price)),
    },
    MarketColumn {
        header: "CACHE/M",
        sort: SortColumn::Cache,
        keyed: true,
        width: 9,
        cell: |row, _ui, _theme| Line::from(format::cache_column(row.cache_read_price)),
    },
    MarketColumn {
        header: "ECI",
        sort: SortColumn::Eci,
        keyed: true,
        width: 7,
        cell: |row, _ui, _theme| Line::from(format::eci_column(row.eci)),
    },
    MarketColumn {
        header: "MODEL",
        sort: SortColumn::Model,
        keyed: true,
        width: 28,
        cell: model_column,
    },
    MarketColumn {
        header: "FLAGS",
        sort: SortColumn::Model,
        keyed: false,
        width: 5,
        cell: |row, _ui, _theme| Line::from(flags_column(row)),
    },
];

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

/// Draws the popup for one frame: table, filter bar, status line,
/// footer, then the `?` help card when it is toggled on.
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

    // The focused filter bar lives in the block title; the plain title
    // shows otherwise.
    let title = if ui.filter_focused {
        filter_title(&ui.filter)
    } else {
        " Model Market ".to_owned()
    };
    let border = if ui.filter_focused {
        theme.picker_highlight_bg
    } else {
        theme.muted_text
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.popup_title)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(border));
    let inner = block.inner(area);

    let header = header_row(&ui);
    let rows: Vec<Row<'static>> = visible
        .iter()
        .enumerate()
        .map(|(idx, row)| table_row(row, idx == ui.selection, &ui, &theme))
        .collect();

    let table = Table::new(
        rows,
        MARKET_COLUMNS.iter().map(|c| Constraint::Length(c.width)),
    )
    .header(header)
    .block(block)
    .column_spacing(1);

    // The highlighted row stays inside the visible window the paging
    // keys clamp against.
    let mut table_state = TableState::default().with_selected(Some(ui.selection));
    frame.render_stateful_widget(table, area, &mut table_state);

    render_status_line(frame, inner, &ui, &data, &theme);
    render_footer(frame, inner, &theme);

    // The help card draws last, over the table behind it — the same
    // order the properties popup's help overlay composes in: surface
    // first, then the card that floats over it.
    if let Some(card) = market_help_card(&ui, area, inner, frame.area(), &theme) {
        card.draw(frame);
    }
}

/// The table header row, with the active sort column marked.
///
/// Derived from [`MARKET_COLUMNS`] so a header cannot sit over the
/// wrong data column, and the marker cannot land on a display-only
/// column.
fn header_row(ui: &MarketUiState) -> Row<'static> {
    let marker = if ui.sort_desc {
        " \u{2193}"
    } else {
        " \u{2191}"
    };
    let cells: Vec<Cell<'static>> = MARKET_COLUMNS
        .iter()
        .map(|col| {
            let text = if col.keyed && col.sort == ui.sort_col {
                format!("{}{marker}", col.header)
            } else {
                col.header.to_owned()
            };
            Cell::from(text).style(Style::default().add_modifier(Modifier::BOLD))
        })
        .collect();
    Row::new(cells)
}

/// One table row: the contract's columns for one listing, with the
/// fuzzy-filter match highlighted in the MODEL column.
fn table_row(
    row: &jinn_model_market_msg::MarketRow,
    selected: bool,
    ui: &MarketUiState,
    theme: &Theme,
) -> Row<'static> {
    let cells: Vec<Cell<'static>> = MARKET_COLUMNS
        .iter()
        .map(|col| Cell::from((col.cell)(row, ui, theme)))
        .collect();
    let mut rendered = Row::new(cells);
    if selected {
        rendered = rendered.style(
            Style::default()
                .fg(theme.primary_text)
                .bg(theme.picker_selected_bg),
        );
    }
    rendered
}

/// The MODEL column as a line: `author/slug` in the row's base style,
/// then the `:variant` suffix dimmed — the card's MODEL entry calls the
/// suffix a dimmed one, so the table renders one. The filter's match
/// highlight rides over both parts: a matched glyph wears the highlight
/// whatever segment it sits in, exactly as the unsplit render did.
fn model_column(
    row: &jinn_model_market_msg::MarketRow,
    ui: &MarketUiState,
    theme: &Theme,
) -> Line<'static> {
    // Built from the fields rather than sliced off `row_name`: the
    // segments are the name's two halves, and `base.len()` is where the
    // name's byte offsets split between them.
    let base = format!("{}/{}", row.author, row.slug);
    let suffix = match &row.variant {
        Some(variant) => format!(":{variant}"),
        None => String::new(),
    };
    let base_len = base.len();
    let dim = Style::default().fg(theme.muted_text);
    let ranges = ui_state::match_ranges(row, ui);

    let mut spans = highlighted_segment(
        &base,
        Style::default(),
        ranges.as_deref(),
        0,
        theme.picker_highlight_bg,
    );
    spans.extend(highlighted_segment(
        &suffix,
        dim,
        ranges.as_deref(),
        base_len,
        theme.picker_highlight_bg,
    ));
    Line::from(spans)
}

/// One segment of the MODEL column as styled spans: `text` starts at
/// byte `name_offset` within the row's display name, so the filter's
/// match ranges (offsets into the whole name) rebasing to the segment
/// is a subtraction. A range cannot straddle a segment boundary — every
/// range is one byte wide and the boundary sits on a character edge.
fn highlighted_segment(
    text: &str,
    base_style: Style,
    ranges: Option<&[Range<usize>]>,
    name_offset: usize,
    highlight_bg: Color,
) -> Vec<Span<'static>> {
    let Some(ranges) = ranges else {
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![Span::styled(text.to_owned(), base_style)]
        };
    };
    if text.is_empty() {
        return Vec::new();
    }
    let segment_end = name_offset + text.len();
    let within: Vec<Range<usize>> = ranges
        .iter()
        .filter(|r| r.start >= name_offset && r.end <= segment_end)
        .map(|r| (r.start - name_offset)..(r.end - name_offset))
        .collect();
    jinn_selection_widget::highlight_text_with_bg(text, base_style, &within, highlight_bg)
        .into_iter()
        .map(|span| Span::styled(span.content.into_owned(), span.style))
        .collect()
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
/// active mix with its `m` hint, the epoch source, and the
/// degraded-mode/error line.
fn render_status_line(
    frame: &mut Frame<'_>,
    inner: Rect,
    ui: &MarketUiState,
    data: &MarketDataState,
    theme: &Theme,
) {
    let status_y = inner.y + inner.height.saturating_sub(2);
    let status_area = Rect::new(inner.x, status_y, inner.width, 1);
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
    spans.push(Span::styled(" \u{b7} [m] adjust", muted));
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

/// The keybind footer on the popup's last inner row, rendered from
/// [`MARKET_BINDINGS`] — the same table the bind test checks, so the
/// footer cannot advertise a key the popup does not bind.
fn render_footer(frame: &mut Frame<'_>, inner: Rect, theme: &Theme) {
    let footer_y = inner.y + inner.height.saturating_sub(1);
    let footer_area = Rect::new(inner.x, footer_y, inner.width, 1);
    let key_style = Style::default().fg(theme.popup_title);
    let desc_style = Style::default().fg(theme.muted_text);
    let mut spans = Vec::new();
    for (idx, (key, desc)) in MARKET_BINDINGS.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled(" \u{b7} ", desc_style));
        }
        spans.push(Span::styled((*key).to_owned(), key_style));
        spans.push(Span::styled(format!(" {desc}"), desc_style));
    }
    frame.render_widget(Line::from(spans), footer_area);
}

/// The popup's result-row count for a frame of the given size: the popup
/// height less both borders, the header, the status line, and the
/// footer. At least 1, so paging never windows against zero.
#[must_use]
pub fn results_viewport(popup: Rect) -> usize {
    let inner = popup.height.saturating_sub(2);
    inner.saturating_sub(3).max(1) as usize
}

/// The focused filter's block title: the text plus a cursor block, so
/// focus is visible even with an empty filter.
#[must_use]
pub fn filter_title(filter: &str) -> String {
    format!(" filter: {filter}\u{2588} ")
}
