//! Gutter line construction — pin icons, selection highlights, wrap padding.

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
    pub cursor_bg_color: Color,
    pub is_included_in_context: bool,
    pub gutter_context_color: Color,
}

/// Build gutter lines for a single entry.
///
/// Each line is two spans: the indicator character (col 0, context fg) and a
/// cursor strip space (col 1, yellow bg when selected+focused). The pin icon
/// first line is an exception — when selected+focused it uses a single span
/// with inverted colors since the double-wide emoji occupies both columns.
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

    let cursor_bg = if ctx.is_selected && ctx.chat_log_active {
        ctx.cursor_bg_color
    } else {
        Color::Reset
    };
    let cursor_style = Style::default().bg(cursor_bg);

    // Pin icon first line: single inverted span when selected+focused.
    let pin_highlight_style = if ctx.is_selected && ctx.is_pinned && ctx.chat_log_active {
        Style::default()
            .fg(ctx.theme.gutter_bg)
            .bg(ctx.cursor_bg_color)
    } else {
        Style::default().fg(indicator_fg).bg(cursor_bg)
    };

    let entry_wrapped: u16 = if ctx.content_width == 0 {
        entry_content_lines.len() as u16
    } else {
        Paragraph::new(entry_content_lines.to_vec())
            .wrap(Wrap { trim: false })
            .line_count(ctx.content_width) as u16
    };

    let indicator_char = "𜺏";
    let cursor_char = " ";

    let mut entry_gutter_lines = Vec::new();
    for (j, _) in entry_content_lines.iter().enumerate() {
        let line = if j == 0 && ctx.is_pinned && ctx.is_selected && ctx.chat_log_active {
            // Pin icon first line, selected+focused: single inverted span (double-wide emoji).
            Line::from(Span::styled("📌".to_owned(), pin_highlight_style))
        } else if j == 0 && ctx.is_pinned {
            // Pin icon first line, not selected+focused: pin emoji + cursor strip.
            Line::from(vec![
                Span::styled("📌".to_owned(), pin_highlight_style),
                Span::styled(cursor_char.to_owned(), cursor_style),
            ])
        } else {
            // Normal line: indicator + cursor strip.
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
                gutter_str.to_string(),
                Style::default().fg(theme.border_unfocused),
            ))
        })
        .collect()
}
