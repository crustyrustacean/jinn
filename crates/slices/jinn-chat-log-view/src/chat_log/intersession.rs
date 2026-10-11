//! Inter-session message rendering — who sent what to whom.
//!
//! The transcript already shows the message's text (the delivery entry is
//! an ordinary run or turn); this kind exists so the log also shows the
//! *route*: the source session, by name when it had one, and the target.
//! The header carries both short ids so a cross-session exchange can be
//! followed from either side's log.

use ratatui::style::Style;
use ratatui::text::Line;

use super::shared::{Pad, RenderContext, multiline_styled, pad_entry};

/// A session id's display form: the short hash, like the rest of the UI.
fn short_id(id: &jinn_core_types::SessionId) -> String {
    id.to_string().chars().take(8).collect()
}

pub fn to_lines(
    source_session: &jinn_core_types::SessionId,
    source_name: Option<&str>,
    target_session: &jinn_core_types::SessionId,
    text: &str,
    ctx: &RenderContext,
) -> Vec<Line<'static>> {
    let text = super::shared::strip_ansi(text);
    let source = source_name
        .map(str::to_owned)
        .unwrap_or_else(|| short_id(source_session));
    let prefix = format!("[↔ {source} → {}] ", short_id(target_session));
    let mut lines = multiline_styled(
        &text,
        &prefix,
        "",
        Style::default().fg(ctx.theme.focus_accent),
    );
    pad_entry(&mut lines, Pad::Both);
    lines
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        reason = "test code"
    )]

    use super::*;
    use jinn_core_types::SessionId;
    use jinn_theme::default_theme;

    fn render_ctx() -> RenderContext {
        RenderContext {
            content_width: 80,
            is_selected: false,
            is_expanded: false,
            tool_entry_max_lines: 20,
            theme: default_theme(),
            paired_status: None,
            is_streaming: false,
            is_waiting_on_subagent: false,
        }
    }

    fn all_text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.clone()))
            .collect()
    }

    #[rstest::rstest]
    fn header_carries_both_session_identities() {
        // Given a message from session A to session B.
        let ctx = render_ctx();
        let source = SessionId::new();
        let target = SessionId::new();

        // When rendering.
        let lines = to_lines(&source, None, &target, "handing off the stage", &ctx);

        // Then the header carries both short ids.
        let text = all_text(&lines);
        let source_short: String = source.to_string().chars().take(8).collect();
        let target_short: String = target.to_string().chars().take(8).collect();
        assert!(
            text.contains(&source_short) && text.contains(&target_short),
            "both identities must render, got {text:?}"
        );
    }

    #[rstest::rstest]
    fn a_named_source_renders_by_name() {
        // Given a source with a display name, using fixed ids so the two
        // sessions cannot share a UUIDv7 millisecond prefix.
        let ctx = render_ctx();
        let source =
            SessionId::try_from_string("aaaaaaa1-0000-0000-0000-00000000000a").expect("fixed id");
        let target =
            SessionId::try_from_string("bbbbbbb2-0000-0000-0000-00000000000b").expect("fixed id");

        // When rendering with the name.
        let lines = to_lines(&source, Some("implementer"), &target, "stage done", &ctx);

        // Then the header names the source and carries the target's short id.
        let text = all_text(&lines);
        assert!(text.contains("implementer"), "name renders: {text:?}");
        assert!(text.contains("bbbbbbb2"), "target id renders: {text:?}");
        assert!(
            !text.contains("aaaaaaa1"),
            "the raw source id is replaced by the name: {text:?}"
        );
    }
}
