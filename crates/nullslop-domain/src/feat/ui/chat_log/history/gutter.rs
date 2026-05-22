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
    pub gutter_active_color: Color,
    pub gutter_inactive_color: Color,
    pub gutter_str: &'a str,
    pub is_included_in_context: bool,
    pub gutter_context_color: Color,
}

/// Build gutter lines for a single entry.
///
/// Handles pin icon on first line, selected/unselected color, pin highlight
/// (inverted colors when pinned + selected), and extra lines for word-wrap
/// overflow.
pub(crate) fn build_entry_gutter_lines(
    entry_content_lines: &[Line<'static>],
    ctx: &GutterStyle<'_>,
) -> Vec<Line<'static>> {
    let gutter_style = if ctx.is_selected && ctx.chat_log_active {
        Style::default().fg(ctx.gutter_active_color)
    } else if ctx.is_selected {
        Style::default().fg(ctx.gutter_inactive_color)
    } else if ctx.is_included_in_context {
        Style::default().fg(ctx.gutter_context_color)
    } else {
        Style::default().fg(ctx.theme.border_unfocused)
    };
    let gutter_content = if ctx.is_pinned {
        "📌"
    } else {
        ctx.gutter_str
    };

    let pin_highlight_style = if ctx.is_selected && ctx.is_pinned && ctx.chat_log_active {
        Style::default()
            .fg(ctx.theme.gutter_bg)
            .bg(ctx.gutter_active_color)
    } else if ctx.is_selected && ctx.is_pinned {
        Style::default()
            .fg(ctx.theme.gutter_bg)
            .bg(ctx.gutter_inactive_color)
    } else {
        Style::default()
    };

    let entry_wrapped: u16 = if ctx.content_width == 0 {
        entry_content_lines.len() as u16
    } else {
        Paragraph::new(entry_content_lines.to_vec())
            .wrap(Wrap { trim: false })
            .line_count(ctx.content_width) as u16
    };

    let mut entry_gutter_lines = Vec::new();
    let blank_gutter = Span::styled(ctx.gutter_str.to_string(), gutter_style);
    for (j, _) in entry_content_lines.iter().enumerate() {
        let span = if j == 0 && ctx.is_pinned {
            Span::styled(gutter_content.to_owned(), pin_highlight_style)
        } else if j == 0 {
            Span::styled(gutter_content.to_owned(), gutter_style)
        } else {
            blank_gutter.clone()
        };
        entry_gutter_lines.push(Line::from(span));
    }

    let logical_count = entry_content_lines.len() as u16;
    if entry_wrapped > logical_count {
        let extra = entry_wrapped - logical_count;
        for _ in 0..extra {
            entry_gutter_lines.push(Line::from(Span::styled(
                ctx.gutter_str.to_string(),
                gutter_style,
            )));
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
