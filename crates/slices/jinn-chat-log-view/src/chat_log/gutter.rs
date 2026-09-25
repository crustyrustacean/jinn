//! Gutter line construction - pin icons, selection highlights, wrap padding.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use jinn_theme::Theme;

/// Context needed to style gutter lines for an entry.
pub struct GutterStyle<'a> {
    pub is_pinned: bool,
    pub is_selected: bool,
    pub chat_log_active: bool,
    pub content_width: u16,
    /// How many rows the entry's content wraps to at `content_width`.
    ///
    /// Supplied by the caller, which already measured this. Measuring it here
    /// would mean re-wrapping (and deep-cloning) the entry's lines a second
    /// time for every visible entry, every frame.
    pub wrapped_count: u32,
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
pub fn build_entry_gutter_lines(
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

    let entry_wrapped = ctx.wrapped_count;

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

    let logical_count = entry_content_lines.len() as u32;
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
pub fn build_blank_gutter_lines(
    count: usize,
    theme: &Theme,
    gutter_str: &str,
) -> Vec<Line<'static>> {
    std::iter::repeat_with(|| {
        Line::from(Span::styled(
            gutter_str.to_owned(),
            Style::default().fg(theme.border_unfocused),
        ))
    })
    .take(count)
    .collect()
}

/// Build a single gutter line for a collapsed ignored block summary.
///
/// Uses gray indicator when not selected, yellow cursor bar when selected.
pub fn build_collapsed_block_gutter_line(
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use jinn_theme::default_theme;

    /// Build a style with only the fields the gutter reads varied.
    fn style(wrapped_count: u32, content_width: u16) -> GutterStyle<'static> {
        GutterStyle {
            is_pinned: false,
            is_selected: false,
            chat_log_active: false,
            content_width,
            wrapped_count,
            theme: Box::leak(Box::new(default_theme())),
            cursor_color: Color::Yellow,
            is_included_in_context: false,
            gutter_context_color: Color::Cyan,
        }
    }

    /// A content line `width` columns wide, so it wraps to a predictable count.
    fn content_lines(n: usize) -> Vec<Line<'static>> {
        (0..n)
            .map(|i| Line::from(format!("line {i} is fairly long text to wrap")))
            .collect()
    }

    #[rstest::rstest]
    fn gutter_emits_one_line_per_content_line_when_not_wrapping() {
        // Given content of 3 logical lines which do not wrap.
        let lines = content_lines(3);

        // When building gutter lines for a wrapped count equal to the line count.
        let gutter = build_entry_gutter_lines(&lines, &style(3, 80));

        // Then one gutter line is produced per content line.
        assert_eq!(gutter.len(), 3);
    }

    #[rstest::rstest]
    fn gutter_pads_out_to_the_wrapped_count() {
        // Given 1 logical content line that wraps to 5 rows.
        let lines = content_lines(1);

        // When building gutter lines.
        let gutter = build_entry_gutter_lines(&lines, &style(5, 20));

        // Then the gutter is padded out to 5 rows so it stays aligned.
        assert_eq!(gutter.len(), 5);
    }

    #[rstest::rstest]
    fn gutter_does_not_shrink_below_the_logical_line_count() {
        // Given 4 logical content lines but a wrapped count below that.
        let lines = content_lines(4);

        // When building gutter lines.
        let gutter = build_entry_gutter_lines(&lines, &style(2, 80));

        // Then one line per logical line is still emitted.
        assert_eq!(
            gutter.len(),
            4,
            "the gutter must not under-produce rows the content occupies"
        );
    }

    #[rstest::rstest]
    fn gutter_zero_wrapped_count_emits_one_line_per_content_line() {
        // Given content lines and a zero wrapped count, as Pass 1 substitutes
        // the logical line count when the content width is zero.
        let lines = content_lines(3);

        // When building gutter lines at content width 0.
        let gutter = build_entry_gutter_lines(&lines, &style(3, 0));

        // Then the gutter matches the logical line count.
        assert_eq!(gutter.len(), 3);
    }

    #[rstest::rstest]
    fn gutter_pinned_selected_first_line_is_the_pin_icon() {
        // Given a pinned, selected entry whose first line is the cursor line.
        let mut ctx = style(1, 80);
        ctx.is_pinned = true;
        ctx.is_selected = true;
        ctx.chat_log_active = true;
        let lines = content_lines(1);

        // When building gutter lines.
        let gutter = build_entry_gutter_lines(&lines, &ctx);

        // Then the first line carries the pin glyph.
        let first: String = gutter[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            first.contains('\u{1F4CC}'),
            "pinned+selected first line should be the pin icon, got {first:?}"
        );
    }
}
