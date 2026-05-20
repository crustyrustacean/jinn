//! Tool call entry rendering — single line with status-based background.
//!
//! Bash tools render as `$ <command>`, all others as `<name> <arguments>`.
//! Background color is determined by the paired tool result's status:
//! no background while pending, green on success, red on failure.

use crate::feat::session::tool_result_status::ToolResultStatus;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::shared::{RenderContext, pad_line_to_width, truncate_to_width};

pub fn to_lines(name: &str, arguments: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let display_text = format_tool_call_display(name, arguments);
    let truncated = truncate_to_width(&display_text, ctx.content_width as usize);

    let fg = ctx.theme.primary_text;
    let bg = status_background(ctx);
    let style = match bg {
        Some(bg_color) => Style::default().fg(fg).bg(bg_color),
        None => Style::default().fg(fg),
    };

    let mut lines = vec![Line::from(Span::styled(truncated, style))];

    // Pad to full content width so the background spans the entire row.
    if let Some(bg_color) = bg {
        pad_line_to_width(
            &mut lines[0],
            ctx.content_width,
            Style::default().bg(bg_color),
        );
    }

    lines
}

/// Format the tool call display string.
///
/// Bash: `$ <command>` (extracted from JSON arguments).
/// Others: `<name> <arguments>` (with unescaped newlines).
fn format_tool_call_display(name: &str, arguments: &str) -> String {
    if name == "bash" {
        let command = extract_bash_command(arguments).unwrap_or_else(|| arguments.to_owned());
        format!("$ {command}")
    } else {
        let arguments = super::shared::unescape_newlines(arguments);
        format!("{name} {arguments}")
    }
}

/// Extract the "command" field from bash JSON arguments.
fn extract_bash_command(arguments: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(arguments).ok()?;
    parsed
        .get("command")?
        .as_str()
        .map(std::borrow::ToOwned::to_owned)
}

/// Determine the background color from the paired result status.
fn status_background(ctx: &RenderContext) -> Option<ratatui::style::Color> {
    match ctx.paired_status? {
        ToolResultStatus::Success => Some(ctx.theme.tool_success_bg),
        ToolResultStatus::Failure => Some(ctx.theme.tool_failure_bg),
        ToolResultStatus::Pending => None,
    }
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
            paired_status: None,
        }
    }

    fn render_context_with_status(status: Option<ToolResultStatus>) -> RenderContext {
        RenderContext {
            content_width: 80,
            _is_selected: false,
            is_expanded: false,
            tool_entry_max_lines: 6,
            theme: crate::feat::theme::default_theme(),
            paired_status: status,
        }
    }

    #[rstest::rstest]
    fn bash_renders_with_dollar_prefix() {
        // Given a bash tool call with a JSON command argument.
        let ctx = render_context(6, false);

        // When converting to lines.
        let lines = to_lines("bash", r#"{"command":"ls -la"}"#, &ctx);

        // Then the first line starts with "$ ".
        let text: String = lines[0].spans.iter().map(|s| s.content.clone()).collect();
        assert!(
            text.starts_with("$ ls -la"),
            "bash tool call should start with '$ ls -la', got: {text}"
        );
    }

    #[rstest::rstest]
    fn non_bash_renders_with_name_args() {
        // Given a non-bash tool call.
        let ctx = render_context(6, false);

        // When converting to lines.
        let lines = to_lines("read", r#"{"path":"/foo.rs"}"#, &ctx);

        // Then the first line starts with "read ".
        let text: String = lines[0].spans.iter().map(|s| s.content.clone()).collect();
        assert!(
            text.starts_with("read "),
            "non-bash tool call should start with 'read ', got: {text}"
        );
    }

    #[rstest::rstest]
    fn tool_call_is_single_line() {
        // Given a tool call with multi-line arguments.
        let ctx = render_context(6, false);
        let args = "line 1\\nline 2\\nline 3";

        // When converting to lines.
        let lines = to_lines("bash", args, &ctx);

        // Then there is exactly one line.
        assert_eq!(
            lines.len(),
            1,
            "tool call should always be a single line, got {}",
            lines.len()
        );
    }

    #[rstest::rstest]
    fn pending_has_no_background() {
        // Given a tool call with no paired result.
        let ctx = render_context_with_status(None);

        // When converting to lines.
        let lines = to_lines("bash", r#"{"command":"ls"}"#, &ctx);

        // Then the line has no background color.
        assert!(
            lines[0].spans.iter().all(|s| s.style.bg.is_none()),
            "pending tool call should have no background"
        );
    }

    #[rstest::rstest]
    fn success_has_green_background() {
        // Given a tool call paired with a successful result.
        let ctx = render_context_with_status(Some(ToolResultStatus::Success));
        let theme = crate::feat::theme::default_theme();

        // When converting to lines.
        let lines = to_lines("bash", r#"{"command":"ls"}"#, &ctx);

        // Then the line has the success background color.
        let has_bg = lines[0]
            .spans
            .iter()
            .any(|s| s.style.bg == Some(theme.tool_success_bg));
        assert!(has_bg, "successful tool call should have green background");
    }

    #[rstest::rstest]
    fn failure_has_red_background() {
        // Given a tool call paired with a failed result.
        let ctx = render_context_with_status(Some(ToolResultStatus::Failure));
        let theme = crate::feat::theme::default_theme();

        // When converting to lines.
        let lines = to_lines("bash", r#"{"command":"ls"}"#, &ctx);

        // Then the line has the failure background color.
        let has_bg = lines[0]
            .spans
            .iter()
            .any(|s| s.style.bg == Some(theme.tool_failure_bg));
        assert!(has_bg, "failed tool call should have red background");
    }

    #[rstest::rstest]
    fn pending_status_has_no_background() {
        // Given a tool call paired with a pending result.
        let ctx = render_context_with_status(Some(ToolResultStatus::Pending));

        // When converting to lines.
        let lines = to_lines("bash", r#"{"command":"ls"}"#, &ctx);

        // Then the line has no background color.
        assert!(
            lines[0].spans.iter().all(|s| s.style.bg.is_none()),
            "pending paired tool call should have no background"
        );
    }

    #[rstest::rstest]
    fn bash_json_parse_failure_fallback() {
        // Given a bash tool call with malformed JSON arguments.
        let ctx = render_context(6, false);

        // When converting to lines.
        let lines = to_lines("bash", "not json at all", &ctx);

        // Then the line still starts with "$ ".
        let text: String = lines[0].spans.iter().map(|s| s.content.clone()).collect();
        assert!(
            text.starts_with("$ "),
            "bash should fallback to raw args on JSON parse failure, got: {text}"
        );
    }

    #[rstest::rstest]
    fn long_command_truncated_to_width() {
        // Given a tool call with a very long command and narrow content width.
        let ctx = RenderContext {
            content_width: 20,
            _is_selected: false,
            is_expanded: false,
            tool_entry_max_lines: 6,
            theme: crate::feat::theme::default_theme(),
            paired_status: None,
        };
        let long_cmd = "a".repeat(100);

        // When converting to lines.
        let lines = to_lines("bash", &format!(r#"{{"command":"{long_cmd}"}}"#), &ctx);

        // Then the line is truncated to 20 graphemes.
        let text: String = lines[0].spans.iter().map(|s| s.content.clone()).collect();
        assert_eq!(
            text.len(),
            20,
            "long command should be truncated to content_width, got {} chars",
            text.len()
        );
    }

    #[rstest::rstest]
    fn uses_primary_text_foreground() {
        // Given a tool call.
        let ctx = render_context(6, false);
        let theme = crate::feat::theme::default_theme();

        // When converting to lines.
        let lines = to_lines("bash", r#"{"command":"ls"}"#, &ctx);

        // Then the line uses primary_text as foreground.
        let has_fg = lines[0]
            .spans
            .iter()
            .any(|s| s.style.fg == Some(theme.primary_text));
        assert!(has_fg, "tool call should use primary_text foreground");
    }
}
