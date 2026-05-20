//! Table entry rendering — aligned columns with bold headers.

use crate::protocol::TableData;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::shared::{Pad, RenderContext, pad_entry, unicode_segementation_display_width};

/// Render a [`TableData`] as aligned, styled lines.
///
/// Builds column widths from headers and rows, then produces:
/// - A bold header line
/// - A separator line
/// - Styled data rows with per-cell coloring
pub fn to_lines(data: &TableData, ctx: &RenderContext) -> Vec<Line<'static>> {
    let num_cols = data.headers.len();
    if num_cols == 0 {
        return vec![Line::from(Span::styled(
            "(empty table)",
            Style::default().fg(ctx.theme.muted_text),
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
        Style::default().fg(ctx.theme.muted_text),
    )));

    // Data rows.
    for row in &data.rows {
        let row_spans = build_row_spans(row, &col_widths, sep, Style::default());
        lines.push(Line::from(row_spans));
    }

    pad_entry(&mut lines, Pad::Both);
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::feat::ui::chat_log::shared::RenderContext;
    use ratatui::style::{Color, Modifier};

    fn render_context() -> RenderContext {
        RenderContext {
            content_width: 60,
            _is_selected: false,
            is_expanded: false,
            tool_entry_max_lines: 5,
            theme: crate::feat::theme::default_theme(),
            paired_status: None,
        }
    }

    fn sample_table() -> TableData {
        TableData {
            headers: vec![
                Span::raw("Provider"),
                Span::raw("Count"),
                Span::raw("Status"),
            ],
            rows: vec![vec![
                Span::raw("ollama"),
                Span::raw("5"),
                Span::styled("\u{2705}", Style::default().fg(Color::Green)),
            ]],
        }
    }

    #[rstest::rstest]
    fn table_header_spans_are_bold() {
        // Given a table with headers.
        let data = sample_table();
        let ctx = render_context();

        // When converting to lines.
        let lines = to_lines(&data, &ctx);

        // Then line 1 (after padding) contains a span with "Provider" that has bold modifier.
        let header_line = &lines[1];
        let provider_span = header_line
            .spans
            .iter()
            .find(|s| s.content == "Provider")
            .expect("should find Provider span");
        assert!(
            provider_span.style.add_modifier.contains(Modifier::BOLD),
            "header span should be bold"
        );
    }

    #[rstest::rstest]
    fn table_data_rows_contain_cell_content() {
        // Given a table with a data row containing "ollama".
        let data = sample_table();
        let ctx = render_context();

        // When converting to lines.
        let lines = to_lines(&data, &ctx);

        // Then some line after padding+header+separator contains "ollama".
        let data_lines = &lines[3..]; // skip pad + header + separator
        let has_ollama = data_lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.starts_with("ollama")));
        assert!(has_ollama, "data row should contain 'ollama'");
    }

    #[rstest::rstest]
    fn table_separator_line_contains_box_drawing_chars() {
        // Given a table with headers and rows.
        let data = sample_table();
        let ctx = render_context();

        // When converting to lines.
        let lines = to_lines(&data, &ctx);

        // Then line 2 (after padding+header) contains the ─ (U+2500) character.
        let separator_line = &lines[2];
        let has_box_drawing = separator_line
            .spans
            .iter()
            .any(|s| s.content.contains('\u{2500}'));
        assert!(has_box_drawing, "separator line should contain ─ (U+2500)");
    }
}
