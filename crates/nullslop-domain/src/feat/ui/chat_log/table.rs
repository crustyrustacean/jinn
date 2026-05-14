//! Table entry rendering — aligned columns with bold headers.

use crate::protocol::TableData;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::shared::{unicode_segementation_display_width, RenderContext};

/// Render a [`TableData`] as aligned, styled lines.
///
/// Builds column widths from headers and rows, then produces:
/// - A bold header line
/// - A separator line
/// - Styled data rows with per-cell coloring
pub fn to_lines(data: &TableData, _ctx: &RenderContext) -> Vec<Line<'static>> {
    let num_cols = data.headers.len();
    if num_cols == 0 {
        return vec![Line::from(Span::styled(
            "(empty table)",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    // Compute column widths: max of header and all row cells.
    let mut col_widths = vec![0usize; num_cols];
    for (i, h) in data.headers.iter().enumerate() {
        col_widths[i] = col_widths[i].max(unicode_segementation_display_width(&h.content));
    }
    for row in &data.rows {
        for (i, cell) in row.iter().enumerate() {
            if i < num_cols {
                col_widths[i] =
                    col_widths[i].max(unicode_segementation_display_width(&cell.content));
            }
        }
    }

    let sep = " │ ";
    let mut lines = Vec::new();

    // Header line.
    let header_spans = build_row_spans(
        &data.headers,
        &col_widths,
        sep,
        Style::default().add_modifier(Modifier::BOLD),
    );
    lines.push(Line::from(header_spans));

    // Separator line.
    let sep_parts: Vec<String> = col_widths.iter().map(|&w| "─".repeat(w)).collect();
    let sep_text = sep_parts.join("─┼─");
    lines.push(Line::from(Span::styled(
        sep_text,
        Style::default().fg(Color::DarkGray),
    )));

    // Data rows.
    for row in &data.rows {
        let row_spans = build_row_spans(row, &col_widths, sep, Style::default());
        lines.push(Line::from(row_spans));
    }

    lines
}

/// Build styled spans for a single table row, padding cells to column width.
fn build_row_spans(
    cells: &[Span<'static>],
    col_widths: &[usize],
    separator: &str,
    default_style: Style,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(separator.to_owned()));
        }
        let width = unicode_segementation_display_width(&cell.content);
        let padding = col_widths
            .get(i)
            .copied()
            .unwrap_or(0)
            .saturating_sub(width);
        // Merge the cell's style with the default style.
        let style = if cell.style == Style::default() {
            default_style
        } else {
            cell.style.patch(default_style)
        };
        spans.push(Span::styled(
            format!("{}{}", cell.content, " ".repeat(padding)),
            style,
        ));
    }
    spans
}
