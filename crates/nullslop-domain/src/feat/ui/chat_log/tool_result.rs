//! Tool result entry rendering.
//!
//! **Collapsed** (default): shows the tool name and last N lines of output,
//! with each line truncated to content width. No word wrapping, no block
//! background padding — minimal escape sequences for streaming output.
//!
//! **Expanded:** shows the full content with word wrapping and block-style
//! background padding (dark green for success, dark red for failure).
//!
//! Collapsed format:
//! ```text
//! <name>
//! ---(N lines hidden above)---
//! <content line N-1>
//! <content line N>
//! ```

use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::feat::theme::Theme;
use crate::feat::tools_actor::truncation::format_size;
use nullslop_provider::tool_types::TruncationMeta;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;

use super::shared::{Pad, RenderContext, pad_entry_with, pad_line_to_width};

pub fn to_lines(
    name: &str,
    content: &str,
    status: ToolResultStatus,
    truncation: Option<&TruncationMeta>,
    ctx: &RenderContext,
) -> Vec<Line<'static>> {
    if ctx.is_expanded {
        to_lines_expanded(name, content, status, truncation, ctx)
    } else {
        to_lines_collapsed(name, content, status, truncation, ctx)
    }
}

/// Collapsed view: last N lines, truncated to content width, no block padding.
fn to_lines_collapsed(
    name: &str,
    content: &str,
    status: ToolResultStatus,
    truncation: Option<&TruncationMeta>,
    ctx: &RenderContext,
) -> Vec<Line<'static>> {
    let fg = status_foreground(status, &ctx.theme);
    let style = Style::default().fg(fg);

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        truncate_to_width(name, ctx.content_width as usize),
        style,
    )));

    // Content lines.
    let text = super::shared::unescape_newlines(content);
    let text = text.trim_start_matches('\n');
    let all_lines: Vec<&str> = text.split('\n').collect();

    let max = ctx.tool_entry_max_lines as usize;
    if all_lines.len() <= max {
        // Short content — show all lines.
        for line_text in &all_lines {
            lines.push(Line::from(Span::styled(
                truncate_to_width(line_text, ctx.content_width as usize),
                style,
            )));
        }
    } else {
        // Long content — show truncation indicator + last N lines.
        let remaining = all_lines.len() - max;
        let indicator = truncate_to_width(
            &format!("---({remaining} lines hidden above)---"),
            ctx.content_width as usize,
        );
        lines.push(Line::from(Span::styled(indicator, style)));
        for line_text in &all_lines[remaining..] {
            lines.push(Line::from(Span::styled(
                truncate_to_width(line_text, ctx.content_width as usize),
                style,
            )));
        }
    }

    // Content-level truncation indicator.
    if let Some(meta) = truncation {
        let label = format_content_truncation_label(meta);
        let indicator_style = Style::default().fg(ctx.theme.focus_accent);
        lines.push(Line::from(Span::styled(
            truncate_to_width(&label, ctx.content_width as usize),
            indicator_style,
        )));
    }

    lines
}

/// Expanded view: full content, word wrapping, block-style background padding.
fn to_lines_expanded(
    name: &str,
    content: &str,
    status: ToolResultStatus,
    truncation: Option<&TruncationMeta>,
    ctx: &RenderContext,
) -> Vec<Line<'static>> {
    let bg = status_background(status, &ctx.theme);
    let style = Style::default().fg(ctx.theme.tool_fg).bg(bg);

    // Name line.
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(name.to_owned(), style)));

    // Content lines.
    let text = super::shared::unescape_newlines(content);
    let text = text.trim_start_matches('\n');
    let all_lines: Vec<&str> = text.split('\n').collect();

    for line_text in &all_lines {
        lines.push(Line::from(Span::styled((*line_text).to_owned(), style)));
    }

    // Content-level truncation indicator.
    if let Some(meta) = truncation {
        let indicator_style = Style::default().fg(ctx.theme.focus_accent).bg(bg);
        let label = format_content_truncation_label(meta);
        lines.push(Line::from(Span::styled(label, indicator_style)));
    }

    // Pad each line to full content width for BLOCK effect.
    for line in &mut lines {
        pad_line_to_width(line, ctx.content_width, Style::default().bg(bg));
    }

    // Add padding above and below with the entry's background.
    let pad_bg = Style::default().bg(bg);
    let pad_line = Line::from(Span::styled(
        " ".repeat(ctx.content_width as usize),
        pad_bg,
    ));
    pad_entry_with(&mut lines, Pad::Both, pad_line);
    lines
}

/// Select the background color based on tool result status.
fn status_background(status: ToolResultStatus, theme: &Theme) -> ratatui::style::Color {
    match status {
        ToolResultStatus::Pending => theme.tool_pending_bg,
        ToolResultStatus::Success => theme.tool_success_bg,
        ToolResultStatus::Failure => theme.tool_failure_bg,
    }
}

/// Select the foreground color based on tool result status.
///
/// Collapsed view uses foreground only (no block background).
fn status_foreground(status: ToolResultStatus, theme: &Theme) -> ratatui::style::Color {
    match status {
        ToolResultStatus::Pending => theme.tool_pending_bg,
        ToolResultStatus::Success => theme.tool_success_bg,
        ToolResultStatus::Failure => theme.tool_failure_bg,
    }
}

/// Truncate a string to `max_width` graphemes.
///
/// Returns the string unchanged if it fits.
/// Returns an empty string if `max_width` is 0.
fn truncate_to_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let graphemes: Vec<&str> = s.graphemes(true).collect();
    if graphemes.len() <= max_width {
        s.to_owned()
    } else {
        graphemes[..max_width].iter().copied().collect()
    }
}

/// Format a human-readable label for content-level truncation.
///
/// Shows how much of the original output is visible, e.g.:
/// `⚠ Output truncated (500 of 2000 lines, 25.0KB of 100.0KB)`
fn format_content_truncation_label(meta: &TruncationMeta) -> String {
    format!(
        "⚠ Output truncated ({} of {} lines, {} of {})",
        meta.output_lines,
        meta.total_lines,
        format_size(meta.output_bytes),
        format_size(meta.total_bytes),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::feat::session::tool_result_status::ToolResultStatus;
    use crate::feat::ui::chat_log::shared::RenderContext;

    fn render_context(max_lines: u16, is_expanded: bool) -> RenderContext {
        RenderContext {
            content_width: 80,
            _is_selected: false,
            is_expanded,
            tool_entry_max_lines: max_lines,
            theme: crate::feat::theme::default_theme(),
        }
    }

    #[rstest::rstest]
    fn truncated_tool_result_shows_indicator() {
        // Given a 10-line tool result with max_lines=5, not expanded.
        let ctx = render_context(5, false);
        let content = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");

        // When converting to lines.
        let lines = to_lines("bash", &content, ToolResultStatus::Success, None, &ctx);
        let has_indicator = lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.content.contains("---(5 lines hidden above)---"))
        });
        assert!(
            has_indicator,
            "truncated tool result should contain '---(5 lines hidden above)---'"
        );
    }

    #[rstest::rstest]
    fn expanded_tool_result_shows_all_lines() {
        // Given a 10-line tool result with max_lines=5, expanded.
        let ctx = render_context(5, true);
        let content = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");

        // When converting to lines.
        let lines = to_lines("bash", &content, ToolResultStatus::Success, None, &ctx);

        // Then some line contains "line 10".
        let has_last_line = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains("line 10")));
        assert!(
            has_last_line,
            "expanded tool result should contain 'line 10'"
        );

        // And no line contains the truncation indicator.
        let has_indicator = lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.content.contains("lines hidden above"))
        });
        assert!(
            !has_indicator,
            "expanded tool result should not show truncation indicator"
        );
    }

    #[rstest::rstest]
    fn short_tool_result_not_truncated() {
        // Given a 3-line tool result with max_lines=5, not expanded.
        let ctx = render_context(5, false);
        let content = "line 1\nline 2\nline 3".to_owned();

        // When converting to lines.
        let lines = to_lines("bash", &content, ToolResultStatus::Success, None, &ctx);

        // Then no line contains the truncation indicator.
        let has_indicator = lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.content.contains("lines hidden above"))
        });
        assert!(!has_indicator, "short tool result should not be truncated");
    }

    #[rstest::rstest]
    fn tool_result_name_on_first_line() {
        // Given a tool result with name "bash" and content "output".
        let ctx = render_context(5, false);

        // When converting to lines.
        let lines = to_lines("bash", "output", ToolResultStatus::Success, None, &ctx);

        // Then line 0 contains "bash".
        let name_content: String = lines[0].spans.iter().map(|s| s.content.clone()).collect();
        assert!(
            name_content.starts_with("bash"),
            "first line should start with tool name"
        );

        // And line 1 contains "output".
        let content_line: String = lines[1].spans.iter().map(|s| s.content.clone()).collect();
        assert!(
            content_line.starts_with("output"),
            "second line should start with content"
        );
    }

    #[rstest::rstest]
    fn tool_result_with_literal_newline_renders_multiple_lines() {
        // Given a tool result with content containing literal \n.
        let ctx = render_context(5, false);

        // When converting to lines.
        let lines = to_lines(
            "bash",
            r"line one\nline two\nline three",
            ToolResultStatus::Success,
            None,
            &ctx,
        );

        // Then the result has multiple lines (name + 3 content lines = 4).
        assert_eq!(
            lines.len(),
            4,
            "tool result with literal \\n should produce 4 lines (name + 3 content), got {}",
            lines.len()
        );

        // And no line contains the literal two-character \n.
        for line in &lines {
            let text: String = line.spans.iter().map(|s| s.content.clone()).collect();
            assert!(
                !text.contains("\\n"),
                "line should not contain literal \\n, got: {text}"
            );
        }
    }

    #[rstest::rstest]
    fn pending_tool_result_uses_pending_foreground() {
        // Given a pending tool result.
        let ctx = render_context(5, false);
        let theme = crate::feat::theme::default_theme();

        // When converting to lines.
        let lines = to_lines("bash", "", ToolResultStatus::Pending, None, &ctx);

        // Then the lines use the pending foreground color.
        assert!(
            !lines.is_empty(),
            "pending tool result should produce lines"
        );
        for line in &lines {
            let has_pending_fg = line
                .spans
                .iter()
                .any(|s| matches!(s.style.fg, Some(color) if color == theme.tool_pending_bg));
            if has_pending_fg {
                return;
            }
        }
        panic!("no line uses the pending foreground color");
    }

    fn sample_truncation_meta() -> TruncationMeta {
        TruncationMeta {
            truncated_by: nullslop_provider::tool_types::TruncatedBy::Lines,
            total_lines: 5000,
            total_bytes: 200_000,
            output_lines: 500,
            output_bytes: 20_000,
        }
    }

    #[rstest::rstest]
    fn content_truncated_tool_result_shows_indicator() {
        // Given a tool result with content-level truncation metadata.
        let ctx = render_context(5, false);
        let meta = sample_truncation_meta();

        // When converting to lines.
        let lines = to_lines(
            "bash",
            "output",
            ToolResultStatus::Success,
            Some(&meta),
            &ctx,
        );

        // Then some line contains the truncation indicator.
        let has_indicator = lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.content.contains("⚠ Output truncated"))
        });
        assert!(
            has_indicator,
            "content-truncated tool result should show truncation indicator"
        );
    }

    #[rstest::rstest]
    fn content_truncated_indicator_contains_line_and_byte_counts() {
        // Given a tool result with specific truncation metadata.
        let ctx = render_context(5, false);
        let meta = sample_truncation_meta();

        // When converting to lines.
        let lines = to_lines(
            "bash",
            "output",
            ToolResultStatus::Success,
            Some(&meta),
            &ctx,
        );

        // Then the indicator contains the expected counts.
        let indicator_line = lines
            .iter()
            .find(|line| {
                line.spans
                    .iter()
                    .any(|s| s.content.contains("⚠ Output truncated"))
            })
            .expect("should have truncation indicator");
        let text: String = indicator_line
            .spans
            .iter()
            .map(|s| s.content.clone())
            .collect();
        assert!(
            text.contains("500 of 5000 lines"),
            "indicator should show output/total lines, got: {text}"
        );
        assert!(
            text.contains("19.5KB of 195.3KB"),
            "indicator should show output/total bytes, got: {text}"
        );
    }

    #[rstest::rstest]
    fn content_truncated_indicator_uses_accent_color() {
        // Given a tool result with content-level truncation metadata.
        let ctx = render_context(5, false);
        let theme = crate::feat::theme::default_theme();
        let meta = sample_truncation_meta();

        // When converting to lines.
        let lines = to_lines(
            "bash",
            "output",
            ToolResultStatus::Success,
            Some(&meta),
            &ctx,
        );

        // Then the indicator line uses the focus_accent color.
        let indicator_line = lines
            .iter()
            .find(|line| {
                line.spans
                    .iter()
                    .any(|s| s.content.contains("⚠ Output truncated"))
            })
            .expect("should have truncation indicator");
        let has_accent_fg = indicator_line
            .spans
            .iter()
            .any(|s| matches!(s.style.fg, Some(color) if color == theme.focus_accent));
        assert!(
            has_accent_fg,
            "truncation indicator should use focus_accent color"
        );
    }

    #[rstest::rstest]
    fn content_truncated_indicator_shows_when_expanded() {
        // Given a tool result with content-level truncation, expanded.
        let ctx = render_context(5, true);
        let meta = sample_truncation_meta();

        // When converting to lines.
        let lines = to_lines(
            "bash",
            "output",
            ToolResultStatus::Success,
            Some(&meta),
            &ctx,
        );

        // Then the indicator still appears (content was truncated at the source).
        let has_indicator = lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.content.contains("⚠ Output truncated"))
        });
        assert!(
            has_indicator,
            "content-truncated indicator should appear even when expanded"
        );
    }

    #[rstest::rstest]
    fn truncated_tool_result_shows_last_lines() {
        // Given a 10-line tool result with max_lines=5, not expanded.
        let ctx = render_context(5, false);
        let content = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");

        // When converting to lines.
        let lines = to_lines("bash", &content, ToolResultStatus::Success, None, &ctx);

        // Then the last 5 lines (6-10) are visible.
        let all_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.clone()))
            .collect();
        for i in 6..=10 {
            assert!(
                all_text.contains(&format!("line {i}")),
                "truncated tool result should contain 'line {i}'"
            );
        }
    }

    #[rstest::rstest]
    fn truncated_tool_result_hides_first_lines() {
        // Given a 10-line tool result with max_lines=5, not expanded.
        let ctx = render_context(5, false);
        let content = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");

        // When converting to lines.
        let lines = to_lines("bash", &content, ToolResultStatus::Success, None, &ctx);

        // Then the first 5 lines (1-5) are not visible.
        let all_text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.clone()))
            .collect();
        for i in 1..=5 {
            assert!(
                !all_text.contains(&format!("line {i}\n"))
                    && !all_text.ends_with(&format!("line {i}")),
                "truncated tool result should not contain 'line {i}'"
            );
        }
    }

    #[rstest::rstest]
    fn non_truncated_tool_result_has_no_content_indicator() {
        // Given a tool result without truncation metadata.
        let ctx = render_context(5, false);

        // When converting to lines.
        let lines = to_lines("bash", "output", ToolResultStatus::Success, None, &ctx);

        // Then no line contains the content truncation indicator.
        let has_indicator = lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.content.contains("⚠ Output truncated"))
        });
        assert!(
            !has_indicator,
            "non-truncated tool result should not show content truncation indicator"
        );
    }

    #[rstest::rstest]
    fn collapsed_long_line_truncated_to_content_width() {
        // Given a tool result with a line longer than content_width.
        let ctx = render_context(5, false);
        let long_line = "a".repeat(100);

        // When converting to lines.
        let lines = to_lines("bash", &long_line, ToolResultStatus::Success, None, &ctx);

        // Then the content line is truncated to content_width (80 graphemes).
        let content_line: String = lines[1].spans.iter().map(|s| s.content.clone()).collect();
        assert_eq!(
            content_line.len(),
            80,
            "content line should be truncated to 80 chars, got {}",
            content_line.len()
        );
    }

    #[rstest::rstest]
    fn collapsed_no_block_padding() {
        // Given a tool result, not expanded.
        let ctx = render_context(5, false);

        // When converting to lines.
        let lines = to_lines("bash", "output", ToolResultStatus::Success, None, &ctx);

        // Then there are no padding lines (just name + content = 2 lines).
        assert_eq!(
            lines.len(),
            2,
            "collapsed tool result should have 2 lines (name + content), got {}",
            lines.len()
        );

        // And no line has a background color set.
        for line in &lines {
            for span in &line.spans {
                assert!(
                    span.style.bg.is_none(),
                    "collapsed line should not have block background, got: {:?}",
                    span.style.bg
                );
            }
        }
    }

    #[rstest::rstest]
    fn expanded_retains_block_padding() {
        // Given a tool result, expanded.
        let ctx = render_context(5, true);

        // When converting to lines.
        let lines = to_lines("bash", "output", ToolResultStatus::Success, None, &ctx);

        // Then there are padding lines (pad + name + content + pad = 4 lines).
        assert_eq!(
            lines.len(),
            4,
            "expanded tool result should have 4 lines (pad + name + content + pad), got {}",
            lines.len()
        );

        // And lines have background color.
        let has_bg = lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.style.bg.is_some())
        });
        assert!(has_bg, "expanded tool result should have block background");
    }
}
