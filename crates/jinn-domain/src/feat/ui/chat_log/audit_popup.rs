//! Audit popup - text rendering for a chat entry's context-change history.
//!
//! Produces the [`Line`]s shown inside the audit popup overlay. Pure function:
//! given a [`ChatEntry`] and a [`Theme`], returns the header line followed by
//! one body line per recorded [`ContextChangeEvent`] (oldest first).
//!
//! Popup geometry and rendering live in the TUI layer; this module owns only
//! the textual content.

use crate::feat::session::chat_entry::{
    ChangeSource, ChatEntry, ContextChangeEvent, ContextOverride,
};
use crate::feat::theme::Theme;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Render the audit popup's text for `entry`.
///
/// Returns a header line followed by one body line per event in `context_history`
/// (chronological order). For entries with no history, returns a header plus a
/// single `(no events recorded)` body line.
///
/// All lines carry the infopopup background color so they render cleanly over
/// the chat-log area underneath.
pub fn format_audit_lines(entry: &ChatEntry, theme: &Theme) -> Vec<Line<'static>> {
    let header_style = Style::default()
        .fg(theme.infopopup_title)
        .bg(theme.infopopup_bg);
    let body_style = Style::default()
        .fg(theme.infopopup_fg)
        .bg(theme.infopopup_bg);

    let count = entry.context_history.len();
    let current = context_override_label(entry.context_override);
    let header = format!("--- audit ({count} events) ({current}) ---");

    let mut lines = Vec::with_capacity(1 + count.max(1));
    lines.push(Line::from(vec![Span::styled(header, header_style)]));

    if entry.context_history.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "(no events recorded)",
            body_style,
        )]));
    } else {
        for event in &entry.context_history {
            lines.push(format_event_line(event, body_style));
        }
    }

    lines
}

/// Format one event as `[source] From -> To`.
fn format_event_line(event: &ContextChangeEvent, style: Style) -> Line<'static> {
    let text = format!(
        "[{}] {} -> {}",
        source_label(&event.source),
        context_override_label(event.from),
        context_override_label(event.to),
    );
    Line::from(vec![Span::styled(text, style)])
}

/// Render the human-readable label for a [`ChangeSource`].
///
/// `User` becomes the literal string `"user"`. `Worker { name }` and
/// `Internal { label }` use the inner string verbatim, with no `worker:` /
/// `internal:` prefix — the bracket convention in the rendered line already
/// communicates "this is a non-user source".
fn source_label(source: &ChangeSource) -> String {
    match source {
        ChangeSource::User => "user".to_string(),
        ChangeSource::Worker { name } => name.clone(),
        ChangeSource::Internal { label } => label.clone(),
    }
}

/// Render the human-readable label for a [`ContextOverride`].
fn context_override_label(o: ContextOverride) -> &'static str {
    match o {
        ContextOverride::Default => "Default",
        ContextOverride::ForcedInclude => "ForcedInclude",
        ContextOverride::ForcedExclude => "ForcedExclude",
    }
}

/// Fixed width of the audit popup, in terminal columns.
///
/// Deliberately constant regardless of terminal size so the popup is always the
/// same shape. See `audit_popup_rect` for placement.
pub const AUDIT_POPUP_WIDTH: u16 = 60;

/// Compute the screen rectangle for the audit popup.
///
/// Placement rules (in priority order):
///
/// 1. **Right-aligned** to the chat-log area: the popup's right edge equals
///    `chat_log_area.x + chat_log_area.width`. If `chat_log_area.width` is
///    smaller than `AUDIT_POPUP_WIDTH`, the popup shrinks to fit and its left
///    edge is clamped to `chat_log_area.x`.
/// 2. **Top edge aligned to `entry_top_y`** (the screen row of the selected
///    entry's first line). If `entry_top_y < chat_log_area.y` (the entry is
///    scrolled above the viewport), the popup pins to `chat_log_area.y`.
/// 3. **Bottom edge clamped to `frame_area.bottom()`**: if the popup would
///    extend below the visible terminal, it slides up so its bottom edge sits
///    exactly at `frame_area.bottom()`. After this clamp, if `popup_y` would
///    drop below `chat_log_area.y`, it is re-clamped to `chat_log_area.y`
///    (popup may overflow the bottom in extremely short terminals — this is
///    accepted as the lesser evil vs. invisible popup).
///
/// `content_line_count` is the number of body lines (header + events or
/// `(no events recorded)`). The popup reserves 2 additional rows for top and
/// bottom borders.
pub fn audit_popup_rect(
    chat_log_area: ratatui::layout::Rect,
    entry_top_y: u16,
    content_line_count: usize,
) -> ratatui::layout::Rect {
    // Width: fixed 60, but never wider than the chat-log area.
    let popup_width = AUDIT_POPUP_WIDTH.min(chat_log_area.width);
    let popup_x = chat_log_area.x + chat_log_area.width.saturating_sub(popup_width);

    // Height: content + top + bottom borders.
    let popup_height = (content_line_count as u16).saturating_add(2);

    // Step 1: pin to entry top, but never above the chat-log area.
    let pinned_top = entry_top_y.max(chat_log_area.y);

    // Step 2: if the popup would extend below the chat-log area, slide it up.
    let area_bottom = chat_log_area.y.saturating_add(chat_log_area.height);
    let mut popup_y = pinned_top;
    let popup_bottom = popup_y.saturating_add(popup_height);
    if popup_bottom > area_bottom {
        popup_y = area_bottom.saturating_sub(popup_height);
    }

    // Step 3: never slide above the chat-log area (last-resort clamp).
    popup_y = popup_y.max(chat_log_area.y);

    ratatui::layout::Rect::new(popup_x, popup_y, popup_width, popup_height)
}

#[cfg(test)]
mod tests {
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::string_slice, clippy::uninlined_format_args, reason = "test code")]
    //! BDD-style tests for `format_audit_lines`.
    //!
    //! Each test asserts exactly one observable behavior of the formatter:
    //! header text, body text per source variant, ordering, or empty-history
    //! fallback.

    use super::*;
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::feat::theme::default_theme;

    /// Convenience: format `entry` with the default theme.
    fn format(entry: &ChatEntry) -> Vec<Line<'static>> {
        format_audit_lines(entry, &default_theme())
    }

    /// Read the unstyled string of a line (for assertions that don't care about color).
    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|s| s.content.clone())
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn format_audit_lines_empty_history_returns_header_and_no_events_line() {
        // Given a default entry (no history, Default override).
        let entry = ChatEntry::user("hi");

        // When formatting.
        let lines = format(&entry);

        // Then the header reflects 0 events and current state.
        assert_eq!(lines.len(), 2);
        assert_eq!(text(&lines[0]), "--- audit (0 events) (Default) ---");
        // And the body is the placeholder.
        assert_eq!(text(&lines[1]), "(no events recorded)");
    }

    #[test]
    fn format_audit_lines_one_user_event_renders_bracketed_source() {
        // Given a user entry that was excluded by the user.
        let mut entry = ChatEntry::user("hi");
        entry.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::User);

        // When formatting.
        let lines = format(&entry);

        // Then the header shows the current override.
        assert_eq!(lines.len(), 2);
        assert_eq!(text(&lines[0]), "--- audit (1 events) (ForcedExclude) ---");
        // And the body line shows the transition with [user] source.
        assert_eq!(text(&lines[1]), "[user] Default -> ForcedExclude");
    }

    #[test]
    fn format_audit_lines_worker_event_uses_name_in_brackets() {
        // Given an entry excluded by the compactor worker.
        let mut entry = ChatEntry::user("hi");
        entry.apply_context_override(
            ContextOverride::ForcedExclude,
            ChangeSource::Worker {
                name: "compactor".to_string(),
            },
        );

        // When formatting.
        let lines = format(&entry);

        // Then the source label is the bare worker name (no `worker:` prefix).
        assert_eq!(text(&lines[1]), "[compactor] Default -> ForcedExclude");
    }

    #[test]
    fn format_audit_lines_internal_event_uses_label_in_brackets() {
        // Given an entry excluded by an internal sweep.
        let mut entry = ChatEntry::user("hi");
        entry.apply_context_override(
            ContextOverride::ForcedExclude,
            ChangeSource::Internal {
                label: "dangling-cleanup".to_string(),
            },
        );

        // When formatting.
        let lines = format(&entry);

        // Then the source label is the bare internal label.
        assert_eq!(
            text(&lines[1]),
            "[dangling-cleanup] Default -> ForcedExclude"
        );
    }

    #[test]
    fn format_audit_lines_multiple_events_preserves_insertion_order() {
        // Given an entry toggled twice (Default -> Exclude -> Default).
        let mut entry = ChatEntry::user("hi");
        entry.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::User);
        entry.apply_context_override(ContextOverride::Default, ChangeSource::User);

        // When formatting.
        let lines = format(&entry);

        // Then 3 lines (header + 2 events) and order matches insertion.
        assert_eq!(lines.len(), 3);
        assert_eq!(text(&lines[1]), "[user] Default -> ForcedExclude");
        assert_eq!(text(&lines[2]), "[user] ForcedExclude -> Default");
    }

    #[test]
    fn format_audit_lines_header_uses_current_override_not_last_event() {
        // Given an entry with one event but the override was reset to Default
        // (so context_history has length 1, current state is Default).
        let mut entry = ChatEntry::user("hi");
        entry.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::User);
        entry.apply_context_override(ContextOverride::Default, ChangeSource::User);

        // When formatting.
        let lines = format(&entry);

        // Then the header parenthetical shows the *current* Default, not the last event's `to`.
        assert!(text(&lines[0]).contains("(Default) ---"));
        // And event count is 2, not 1.
        assert!(text(&lines[0]).contains("(2 events)"));
    }

    mod rect_tests {
        //! Unit tests for `audit_popup_rect`.
        //!
        //! Each test pins one placement rule: in-viewport, scrolled-above,
        //! overflow-bottom, fits-exactly, right-edge alignment, very-small terminal.

        use super::*;
        use ratatui::layout::Rect;

        /// Helper to construct a typical chat-log layout (no separate frame).
        fn layout() -> Rect {
            // 100x40 chat-log area.
            Rect::new(0, 0, 100, 40)
        }

        #[test]
        fn audit_popup_rect_in_viewport_top_aligns_to_entry() {
            // Given entry at row 10, content has 3 lines, area is 40 rows tall.
            let chat = layout();

            // When computing the rect.
            let rect = audit_popup_rect(chat, 10, 3);

            // Then the popup top is pinned to the entry's row.
            assert_eq!(rect.y, 10);
            // And the popup height is content (3) + 2 borders.
            assert_eq!(rect.height, 5);
            // And the popup width is the constant 60.
            assert_eq!(rect.width, AUDIT_POPUP_WIDTH);
            // And the popup right edge equals the chat-log right edge.
            assert_eq!(rect.x + rect.width, chat.x + chat.width);
        }

        #[test]
        fn audit_popup_rect_entry_scrolled_above_viewport_pins_to_chat_top() {
            // Given entry_top_y is above the chat-log top (scrolled off-screen).
            // When computing with entry_top_y = 0 but chat_log_area.y = 5.
            let chat_offset = Rect::new(0, 5, 100, 35);
            let rect = audit_popup_rect(chat_offset, 0, 3);

            // Then the popup pins to the chat-log top, not above it.
            assert_eq!(rect.y, chat_offset.y);
        }

        #[test]
        fn audit_popup_rect_overflow_bottom_slides_up() {
            // Given entry near the bottom and a tall popup that would overflow.
            let chat = layout();
            // Entry at row 35, popup needs 8 rows (content 6 + 2 borders).
            // area_bottom = 40, so popup would render at 35..43 — overflow by 3.
            let rect = audit_popup_rect(chat, 35, 6);

            // Then popup slides up so its bottom edge equals area bottom.
            assert_eq!(rect.y + rect.height, chat.y + chat.height);
            // And the popup y is 32 (40 - 8).
            assert_eq!(rect.y, 32);
        }

        #[test]
        fn audit_popup_rect_fits_exactly_does_not_slide() {
            // Given entry at a row where the popup fits exactly to area bottom.
            let chat = layout();
            // 3 content lines + 2 borders = 5 height. Entry at row 35 → 35+5=40 = area_bottom.
            let rect = audit_popup_rect(chat, 35, 3);

            // Then no slide; popup top equals entry top.
            assert_eq!(rect.y, 35);
            assert_eq!(rect.y + rect.height, chat.y + chat.height);
        }

        #[test]
        fn audit_popup_rect_right_edge_aligns_to_chat_log_right_edge() {
            // Given a chat-log area offset from the frame's left edge (sidebar takes 20 cols).
            let chat = Rect::new(20, 0, 80, 40);

            // When computing the rect.
            let rect = audit_popup_rect(chat, 5, 2);

            // Then popup right edge = chat right edge = 100.
            assert_eq!(rect.x + rect.width, chat.x + chat.width);
            assert_eq!(rect.x + rect.width, 100);
            // And popup left edge = 100 - 60 = 40.
            assert_eq!(rect.x, 40);
        }

        #[test]
        fn audit_popup_rect_chat_log_narrower_than_60_shrinks_popup_width() {
            // Given a chat-log area only 50 columns wide.
            let chat = Rect::new(0, 0, 50, 40);
            // When computing the rect.
            let rect = audit_popup_rect(chat, 5, 2);

            // Then popup width shrinks to chat width (50), not the constant 60.
            assert_eq!(rect.width, 50);
            // And popup left edge is at chat left edge.
            assert_eq!(rect.x, chat.x);
        }
    }
}
