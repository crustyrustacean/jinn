//! The model-market popup's overlay rendering: the centered full-table
//! geometry, the table itself, the filter bar, the status line, the
//! keybind footer, the highlighted model's detail pane, and the `?`
//! help card the view draws below itself while it is toggled on.
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
use jinn_model_market_msg::MarketDetail;
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
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;

use crate::format;
use crate::help::market_help_card;
use crate::routes::MARKET_BINDINGS;
use crate::ui_state;

/// The rows the table always keeps visible above the detail pane, so a
/// pane can never swallow the table whole.
const TABLE_KEEP_ROWS: u16 = 4;
/// The rows the status line and footer occupy at the popup's bottom —
/// the pane's floor sits above them.
const STATUS_ROWS: u16 = 2;

#[cfg(test)]
#[path = "view_tests.rs"]
mod view_tests;

#[cfg(test)]
#[path = "view_detail_tests.rs"]
mod view_detail_tests;

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

    // The detail pane is the highlighted row's second region under the
    // table — the contract's sketch. It draws before the status line
    // because its budget is the rows above the status line.
    let detail = detail_for_highlight(&data, &ui);
    if let Some((detail, eci)) = &detail {
        render_detail_pane(frame, inner, detail, *eci, &theme);
    }

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

/// The pane's data iff the cell's detail belongs to the highlighted row:
/// the detail plus the row's ECI score and confidence interval.
///
/// The cell can hold detail for a model the user has already navigated
/// away from — a `j`/`k` crossed rows faster than the debounce refetched
/// — and a pane showing the previous row's endpoints under the new
/// highlight is a lie on screen. The stale rows are dropped here, at the
/// only place that knows which row is highlighted.
type PaneData = (MarketDetail, Option<(f64, Option<(f64, f64)>)>);

fn detail_for_highlight(data: &MarketDataState, ui: &MarketUiState) -> Option<PaneData> {
    let detail = data.detail.as_ref()?;
    let highlighted = ui_state::highlighted_row(data, ui)?;
    (detail.model_id == highlighted.id).then(|| {
        (
            detail.clone(),
            highlighted.eci.map(|eci| (eci, highlighted.eci_ci)),
        )
    })
}

/// The pane's budget above the status line: the header, the rows the
/// table keeps, and the pane's own lines. The pane never asks for a
/// fixed share of the popup — it takes what its content needs, up to a
/// floor that leaves the table usable, so a one-endpoint model loses no
/// table rows a three-endpoint model keeps.
fn detail_pane_budget(inner: Rect, lines: usize) -> u16 {
    let pane_height = inner.height.saturating_sub(1 + TABLE_KEEP_ROWS);
    u16::try_from(lines)
        .unwrap_or(u16::MAX)
        .min(pane_height)
        .max(1)
}

/// The highlighted model's detail pane: endpoint rows, the model's ECI
/// ± CI, and the pricing-override rows, laid out under the table and
/// above the status line.
///
/// The pane is an overlay, not a table region: the table keeps its own
/// geometry, and the pane paints over the rows its budget allows,
/// transparently sharing the table's grid so column positions line up
/// with the header above. Clear first — the table's selected-row
/// background underneath must not bleed through the pane's own.
fn render_detail_pane(
    frame: &mut Frame<'_>,
    inner: Rect,
    detail: &MarketDetail,
    eci: Option<(f64, Option<(f64, f64)>)>,
    theme: &Theme,
) {
    let lines = detail_lines(detail, eci, theme);
    let height = detail_pane_budget(inner, lines.len());
    let area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(height + STATUS_ROWS),
        inner.width,
        height,
    );
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines), area);
}

/// One endpoint's line: `name (tag)  IN $x/M  OUT $x/M  CACHE —/$x/M  LAT
/// p50 —/Nms  TP p50 —/N tok/s  UP 1d N%`.
///
/// Columns are space-joined fragments, not a `Table`: the pane is an
/// overlay over the table's own grid and a second constraint set would
/// fight the header's. The fragment widths are padded by the paragraph's
/// alignment, and each fragment carries its own style.
fn endpoint_line(
    endpoint: &jinn_model_market_msg::MarketEndpointRow,
    theme: &Theme,
) -> Line<'static> {
    let muted = Style::default().fg(theme.muted_text);
    let value = Style::default().fg(theme.primary_text);
    let lat = endpoint
        .latency_p50_ms
        .map_or_else(|| "\u{2014}".to_owned(), |ms| format!("{ms:.0}ms"));
    let tp = endpoint
        .throughput_p50
        .map_or_else(|| "\u{2014}".to_owned(), |t| format!("{t:.0} tok/s"));
    let uptime = endpoint.uptime_1d.map_or_else(
        || "\u{2014}".to_owned(),
        |frac| format!("{:.1}%", frac * 100.0),
    );
    let tag = if endpoint.tag.is_empty() {
        String::new()
    } else {
        format!(" ({})", endpoint.tag)
    };
    Line::from(vec![
        Span::styled(
            format!("  {}{tag}", endpoint.provider_name),
            Style::default().fg(theme.popup_title),
        ),
        Span::styled("  IN ", muted),
        Span::styled(format::price_per_million(endpoint.prompt_price), value),
        Span::styled("  OUT ", muted),
        Span::styled(format::price_per_million(endpoint.completion_price), value),
        Span::styled("  CACHE ", muted),
        Span::styled(format::cache_column(endpoint.cache_read_price), value),
        Span::styled("  LAT p50 ", muted),
        Span::styled(lat, value),
        Span::styled("  TP p50 ", muted),
        Span::styled(tp, value),
        Span::styled("  UP 1d ", muted),
        Span::styled(uptime, value),
    ])
}

/// The model's ECI ± CI line, or `None` when the row is unmatched — an
/// absent ECI is already stated by the table's `#` flag and the `—` in
/// its ECI column; the pane does not repeat it.
fn eci_line(eci: Option<(f64, Option<(f64, f64)>)>, theme: &Theme) -> Option<Line<'static>> {
    let (eci, ci_interval) = eci?;
    let muted = Style::default().fg(theme.muted_text);
    let ci = ci_interval.map_or_else(String::new, |(low, high)| {
        format!(" \u{b1} [{low:.1}, {high:.1}]")
    });
    Some(Line::from(vec![
        Span::styled("  ECI", muted),
        Span::styled(
            format!(" {eci:.1}{ci}"),
            Style::default().fg(theme.primary_text),
        ),
    ]))
}

/// One pricing-override row: `>Nk ctx: in X / out Y / cch Z`.
///
/// Thresholds are `Option` (the upstream publishes rows without one);
/// the tokens render in `k` when present, and an absent threshold reads
/// as `*` — "any prompt above the endpoint's base price" is a row the
/// pane must not silently drop.
fn override_line(
    (min_prompt_tokens, prompt, completion): (Option<u32>, f64, f64),
) -> Line<'static> {
    let threshold = min_prompt_tokens.map_or_else(
        || "*".to_owned(),
        |tokens| format!(">{}k ctx", tokens / 1_000),
    );
    Line::from(format!(
        "    {threshold}: in {} / out {} / cch \u{2014}",
        format::price_per_million(prompt),
        format::price_per_million(completion),
    ))
}

/// The pane's lines: the model header, one line per endpoint, the ECI
/// line, then the override rows.
fn detail_lines(
    detail: &MarketDetail,
    eci: Option<(f64, Option<(f64, f64)>)>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![header_line(detail, theme)];
    for endpoint in &detail.endpoints {
        lines.push(endpoint_line(endpoint, theme));
    }
    if let Some(eci) = eci_line(eci, theme) {
        lines.push(eci);
    }
    let overrides: Vec<Line<'static>> = detail
        .endpoints
        .iter()
        .flat_map(|endpoint| endpoint.overrides.iter().map(|&row| override_line(row)))
        .collect();
    lines.extend(overrides);
    lines
}

/// The pane's header: the model id the detail was fetched for, so the
/// pane states whose endpoints these are, in the accent.
fn header_line(detail: &MarketDetail, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {} endpoints", detail.model_id),
        Style::default()
            .fg(theme.popup_title)
            .add_modifier(Modifier::BOLD),
    ))
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
