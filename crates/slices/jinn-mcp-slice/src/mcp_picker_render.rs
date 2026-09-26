// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//! The MCP server inspector's overlay rendering.
//!
//! Two panes share one popup: the server list on the left, and a preview on
//! the right that flips between live logs and the server's advertised tools.
//! The live half (connection status, stderr tail, tool list) is refreshed
//! from the MCP runtime and tool registry cells during the render pass, so
//! the inspector keeps ticking without the picker owning a poll loop.

use jinn_mcp_msg::McpPreviewMode;
use jinn_mcp_msg::McpServerEntry;
use jinn_mcp_msg::mcp_picker_slot;
use jinn_picker::RowCtx;
use jinn_picker::picker_style::dim_style;
use jinn_picker::picker_style::split_match_indices;
use jinn_slices::RenderFacts;
use jinn_slices::cell::TypedCell;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::mcp_picker_actions;
use crate::mcp_picker_viewport;

/// The inspector's cell — the single home for what it shows.
#[allow(dead_code, reason = "documents the cell type the render reads")]
type McpPickerCell = TypedCell<jinn_mcp_msg::McpPickerState>;

/// The popup rect for the inspector.
#[must_use]
pub fn mcp_picker_overlay_rect(area: &Rect) -> Option<Rect> {
    Some(jinn_selection_widget::compute_popup_rect(*area))
}

/// The colors the inspector draws with.
#[must_use]
pub fn mcp_picker_palette(theme: &jinn_theme::Theme) -> jinn_picker::Palette {
    jinn_picker::Palette {
        border: ratatui::style::Color::DarkGray,
        filter_text: ratatui::style::Color::White,
        separator: ratatui::style::Color::DarkGray,
        footer: ratatui::style::Color::DarkGray,
        highlight_bg: ratatui::style::Color::DarkGray,
        muted_text: theme.muted_text,
        accent_action: theme.accent_action,
        popup_title: theme.popup_title,
        primary_text: theme.primary_text,
    }
}

/// The inspector's declared binds, in footer order.
#[must_use]
pub fn mcp_picker_binds() -> Vec<jinn_picker::BindRow> {
    crate::mcp_picker_routes::MCP_PICKER_BINDINGS
        .iter()
        .map(|(notation, label)| jinn_picker::BindRow {
            notation,
            label,
            category_hint: "input",
        })
        .collect()
}

/// Renders one picker row: the ✓/✗ enablement marker, the server name, and
/// an em-dash launch description. Filter matches highlight within the name
/// and the description separately — byte offsets from the search text
/// `"{name} {description}"` split at the separator.
pub fn mcp_server_row(entry: &McpServerEntry, ctx: &RowCtx<'_>) -> Line<'static> {
    let style = if ctx.is_selected {
        Style::default()
            .fg(entry.theme.primary_text)
            .bg(entry.theme.picker_selected_bg)
    } else {
        Style::default()
    };

    let (marker, marker_color) = if entry.enabled {
        ("\u{2713} ", entry.theme.focus_accent) // ✓
    } else {
        ("\u{2717} ", entry.theme.error_text) // ✗
    };
    let marker_span = Span::styled(marker.to_owned(), Style::default().fg(marker_color));

    // Match ranges are byte offsets into "{name} {description}"; the space
    // separator sits at byte offset `name_len`.
    let (name_indices, desc_indices) = split_match_indices(ctx.match_ranges, entry.name.len());

    let name_spans = jinn_selection_widget::highlight_text_with_bg(
        &entry.name,
        style,
        &name_indices,
        entry.theme.picker_highlight_bg,
    );

    // Separator and description in dim style.
    let desc_style = dim_style(ctx.is_selected, &entry.theme);
    let sep_span = Span::styled(" \u{2014} ".to_owned(), desc_style);
    let desc_spans = jinn_selection_widget::highlight_text_with_bg(
        &entry.description,
        desc_style,
        &desc_indices,
        entry.theme.picker_highlight_bg,
    );

    let mut spans = vec![marker_span];
    spans.extend(name_spans);
    spans.push(sep_span);
    spans.extend(desc_spans);
    Line::from(spans)
}

/// Renders the preview pane for the selected server: live logs (a status
/// badge line followed by the wrapped stderr tail) or the advertised tools
/// (`name — description`), per the entry's `preview_mode`.
pub fn mcp_picker_preview(
    entry: &McpServerEntry,
    ctx: &jinn_picker::PreviewCtx<'_>,
) -> Vec<Line<'static>> {
    match entry.preview_mode {
        McpPreviewMode::Logs => logs_preview(entry, ctx.width),
        McpPreviewMode::Tools => tools_preview(entry),
    }
}

/// Logs pane: a status badge line followed by the stderr tail,
/// soft-wrapped so long lines don't overflow the preview width.
fn logs_preview(entry: &McpServerEntry, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(status_badge_line(entry));
    if entry.stderr_tail.trim().is_empty() {
        lines.push(
            Line::from("(no stderr yet)".to_owned())
                .style(Style::default().fg(entry.theme.muted_text)),
        );
    } else {
        for raw in entry.stderr_tail.lines() {
            lines.extend(wrap_line(raw, width, entry.theme.primary_text));
        }
    }
    lines
}

/// Tools pane: one line per advertised tool (`name — description`).
fn tools_preview(entry: &McpServerEntry) -> Vec<Line<'static>> {
    if entry.tools.is_empty() {
        return vec![
            Line::from("(no tools advertised)".to_owned())
                .style(Style::default().fg(entry.theme.muted_text)),
        ];
    }
    entry
        .tools
        .iter()
        .map(|(name, desc)| {
            Line::from(vec![
                Span::styled(name.clone(), Style::default().fg(entry.theme.primary_text)),
                Span::styled(
                    format!(" \u{2014} {desc}"),
                    Style::default().fg(entry.theme.muted_text),
                ),
            ])
        })
        .collect()
}

/// One styled line: `Status: running` colored by the live state.
fn status_badge_line(entry: &McpServerEntry) -> Line<'static> {
    let (label, color) = match entry.status {
        None => ("disabled", entry.theme.muted_text),
        Some(jinn_mcp_msg::McpConnectionStatus::Starting) => {
            ("starting", ratatui::style::Color::Yellow)
        }
        Some(jinn_mcp_msg::McpConnectionStatus::Running) => {
            ("running", ratatui::style::Color::Green)
        }
        Some(jinn_mcp_msg::McpConnectionStatus::Dead) => ("dead", ratatui::style::Color::Red),
    };
    Line::from(vec![
        Span::styled(
            "Status: ".to_owned(),
            Style::default().fg(entry.theme.muted_text),
        ),
        Span::styled(label.to_owned(), Style::default().fg(color)),
    ])
}

/// Greedily wraps `raw` to `width` columns, returning one styled line per
/// chunk. Guards against a zero width by treating it as 1 so we never loop
/// forever on an empty/negative-pane edge case.
fn wrap_line(raw: &str, width: usize, color: ratatui::style::Color) -> Vec<Line<'static>> {
    use unicode_segmentation::UnicodeSegmentation;

    let cap = width.max(1);
    let style = Style::default().fg(color);
    let mut out = Vec::new();
    let mut buf = String::new();
    for grapheme in raw.graphemes(true) {
        if buf.graphemes(true).count() >= cap {
            out.push(Line::from(buf.clone()).style(style));
            buf.clear();
        }
        buf.push_str(grapheme);
    }
    if !buf.is_empty() || out.is_empty() {
        out.push(Line::from(buf).style(style));
    }
    out
}

/// The status line: the live enabled count, e.g. `3/7 enabled`.
#[must_use]
pub fn mcp_picker_status(enabled: usize, total: usize, theme: &jinn_theme::Theme) -> Line<'static> {
    Line::from(Span::styled(
        format!("{enabled}/{total} enabled"),
        Style::default().fg(theme.muted_text),
    ))
}

/// Draws the inspector popup for one frame.
pub fn render_mcp_picker(frame: &mut Frame<'_>, area: Rect, facts: &RenderFacts) {
    let Some(cell) = facts.slices.reader(&mcp_picker_slot()) else {
        return;
    };

    // Publish the measured row count for the navigation keys.
    let measured = mcp_picker_viewport::results_viewport(&area);
    cell.update(|state: &mut jinn_mcp_msg::McpPickerState| state.results_viewport = measured);

    // Refresh the highlighted server's live half from the MCP runtime and
    // the tool registry. This is the slice-owned equivalent of the kernel's
    // per-frame inspector pre-pass: the inspector keeps ticking while open,
    // with no poll loop and no app-state access.
    mcp_picker_actions::refresh_inspector_snapshot(&cell, facts);

    let guard = cell.read();
    let state: &jinn_mcp_msg::McpPickerState = &guard;

    let theme = &facts.theme;
    let palette = mcp_picker_palette(theme);
    let keybind =
        jinn_picker::keybind_line(&mcp_picker_binds(), jinn_picker::Tail::Standard, &palette);

    let enabled = mcp_picker_actions::enabled_count(state);
    let total = state.selection.items().len();
    let footers = vec![
        mcp_picker_status(enabled, total, theme),
        Line::from(keybind.0),
    ];

    // `PreviewSelectionWidget` because the inspector has a preview pane; the
    // pane itself comes from the item hooks, and the scroll offset stays at
    // zero — this pane has no scroll mechanism, so it holds still across
    // cursor moves rather than jumping back to the top each time.
    jinn_selection_widget::PreviewSelectionWidget::new(&state.selection)
        .title(Line::from(" MCP Servers "))
        .title_style(Style::default().fg(theme.popup_title))
        .footers(footers)
        .colors(palette.selection_colors())
        .preview_scroll(state.preview_scroll)
        .render(frame, area);
}
