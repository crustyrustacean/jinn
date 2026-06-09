//! Gutter line construction - pin icons, selection highlights, wrap padding.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::feat::theme::Theme;

/// Context needed to style gutter lines for an entry.
pub(crate) struct GutterStyle<'a> {
    pub is_pinned: bool,
    pub is_selected: bool,
    pub chat_log_active: bool,
    pub content_width: u16,
    pub theme: &'a Theme,
    pub cursor_color: Color,
    pub is_included_in_context: bool,
    pub gutter_context_color: Color,
}

/// Build gutter lines for a single entry.
///
/// Each line is two spans: the indicator character (col 0, context fg) and a
/// cursor bar in col 1. The bar only appears when selected+focused - otherwise
/// col 1 is a plain space. The pin icon first line is an exception: when
/// selected+focused, it gets yellow bg and the pin emoji occupies both columns
/// as a single span.
pub(crate) fn build_entry_gutter_lines(
    entry_content_lines: &[Line<'static>],
    ctx: &GutterStyle<'_>,
) -> Vec<Line<'static>> {
    let indicator_fg = if ctx.is_included_in_context {
        ctx.gutter_context_color
    } else {
        ctx.theme.border_unfocused
    };

    let indicator_style = Style::default().fg(indicator_fg);

    let has_cursor = ctx.is_selected && ctx.chat_log_active;

    // Cursor bar: ┃ with yellow fg when cursor present, plain space otherwise.
    let (cursor_char, cursor_style) = if has_cursor {
        ("┃", Style::default().fg(ctx.cursor_color))
    } else {
        (" ", Style::default())
    };

    let entry_wrapped: u16 = if ctx.content_width == 0 {
        entry_content_lines.len() as u16
    } else {
        Paragraph::new(entry_content_lines.to_vec())
            .wrap(Wrap { trim: false })
            .line_count(ctx.content_width) as u16
    };

    let indicator_char = "𜺏";

    let mut entry_gutter_lines = Vec::new();
    for (j, _) in entry_content_lines.iter().enumerate() {
        let line = if j == 0 && ctx.is_pinned && has_cursor {
            // Pin icon first line, selected+focused: yellow bg (double-wide emoji).
            let pin_style = Style::default()
                .fg(ctx.theme.gutter_bg)
                .bg(ctx.cursor_color);
            Line::from(Span::styled("📌".to_owned(), pin_style))
        } else if j == 0 && ctx.is_pinned {
            // Pin icon first line, no cursor: pin emoji with context fg + bar/space.
            let pin_style = Style::default().fg(indicator_fg);
            Line::from(vec![
                Span::styled("📌".to_owned(), pin_style),
                Span::styled(cursor_char.to_owned(), cursor_style),
            ])
        } else {
            // Normal line: indicator + cursor bar/space.
            Line::from(vec![
                Span::styled(indicator_char.to_owned(), indicator_style),
                Span::styled(cursor_char.to_owned(), cursor_style),
            ])
        };
        entry_gutter_lines.push(line);
    }

    let logical_count = entry_content_lines.len() as u16;
    if entry_wrapped > logical_count {
        let extra = entry_wrapped - logical_count;
        for _ in 0..extra {
            entry_gutter_lines.push(Line::from(vec![
                Span::styled(indicator_char.to_owned(), indicator_style),
                Span::styled(cursor_char.to_owned(), cursor_style),
            ]));
        }
    }

    entry_gutter_lines
}

/// Build blank gutter spacer lines for bottom-alignment padding.
pub(crate) fn build_blank_gutter_lines(
    count: usize,
    theme: &Theme,
    gutter_str: &str,
) -> Vec<Line<'static>> {
    (0..count)
        .map(|_| {
            Line::from(Span::styled(
                gutter_str.to_owned(),
                Style::default().fg(theme.border_unfocused),
            ))
        })
        .collect()
}

/// Build a single gutter line for a collapsed ignored block summary.
///
/// Uses gray indicator when not selected, yellow cursor bar when selected.
pub(crate) fn build_collapsed_block_gutter_line(
    is_selected: bool,
    chat_log_active: bool,
    theme: &Theme,
    cursor_color: Color,
) -> Line<'static> {
    let indicator_style = Style::default().fg(theme.border_unfocused);
    let has_cursor = is_selected && chat_log_active;

    if has_cursor {
        let cursor_style = Style::default().fg(cursor_color);
        Line::from(vec![
            Span::styled("…".to_owned(), indicator_style),
            Span::styled("┃".to_owned(), cursor_style),
        ])
    } else {
        Line::from(vec![
            Span::styled("…".to_owned(), indicator_style),
            Span::styled(" ".to_owned(), Style::default()),
        ])
    }
}
