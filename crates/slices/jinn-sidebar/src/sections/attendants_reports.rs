//! Attendant reports popup — the hover preview for the Attendants section.
//!
//! Rendered as a bordered floating surface while the sidebar's Attendants
//! section is focused. Anchored to the highlighted attendant's row, right
//! edge aligned with the terminal edge, showing the attendant's last 10
//! reports newest last (reading order: oldest first).
//!
//! Like the session preview this overflows the sidebar column, so it paints
//! from the floating-surfaces layer rather than the column's own draw.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;

use crate::sections::attendants_section;
use jinn_kernel::common::app_state::AppState;
use jinn_slices::DrawContext;

/// The maximum number of reports the popup shows — older ones are dropped
/// from the top so the newest, most relevant report is always visible.
const MAX_REPORTS: usize = 10;
/// Minimum popup width in cells; below this the frame is too narrow and the
/// popup is skipped rather than drawn unreadably small.
const MIN_POPUP_WIDTH: u16 = 30;
/// Minimum popup height in cells — the empty state plus borders must stay
/// visible even when the popup has to slide up.
const MIN_POPUP_HEIGHT: u16 = 5;

/// Renders the attendant-reports popup when the sidebar Attendants section
/// is focused.
///
/// - `sidebar_rect`: the full sidebar column rect
/// - `frame_area`: the total frame area (used for right-edge alignment)
pub fn render_attendant_reports_for_state(
    frame: &mut Frame<'_>,
    sidebar_rect: Rect,
    frame_area: Rect,
    ctx: &dyn DrawContext<AppState>,
) {
    let state = ctx.state();
    if state.frontend.sidebar_section() != Some(jinn_sidebar_msg::SidebarSectionId::Attendant) {
        return;
    }
    let Some(id) = state
        .frontend
        .with_sections(|s| s.attendant.selected_id.clone(), || None)
    else {
        return;
    };
    let Some(row) = jinn_attendant::section_rows::attendant_rows(state)
        .into_iter()
        .find(|row| row.session_id == id)
    else {
        return;
    };
    let Some(session) = state.session.get(&row.session_id) else {
        return;
    };
    let reports = session.attendant_reports();
    let theme = &state.frontend.theme;

    // Anchor the popup to the attendant's own row through the same document
    // layout the sidebar renders with, so it stays attached while scrolling.
    // The section-relative row offset comes from the section itself, not the
    // attendant's list index: attendants render two lines each, so anchoring
    // to the index would pin every attendant after the first to the section
    // header.
    let cursor_y = crate::sections::layout::frame_row_of(
        sidebar_rect,
        state,
        ctx.config(),
        jinn_sidebar_msg::SidebarSectionId::Attendant,
        crate::sections::layout::cursor_row_in_section(
            state,
            jinn_sidebar_msg::SidebarSectionId::Attendant,
        )
        .unwrap_or(0),
    );

    let shown = reports
        .len()
        .saturating_sub(reports.len().saturating_sub(MAX_REPORTS));
    let Some(popup_rect) = popup_rect(frame_area, sidebar_rect, cursor_y, shown) else {
        return;
    };

    let mut lines = Vec::new();
    let newest_first = reports.iter().rev().take(MAX_REPORTS).collect::<Vec<_>>();
    if newest_first.is_empty() {
        lines.push(Line::from(Span::styled(
            attendants_section::NEVER_REPORTED_MARKER,
            Style::default().fg(theme.dormant_fg),
        )));
    }
    for report in newest_first {
        lines.push(Line::from(vec![
            Span::styled(
                format!("#{} ", report.run),
                Style::default()
                    .fg(theme.attendant_fg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                report.published_at.strftime("%Y-%m-%d %H:%M").to_string(),
                Style::default().fg(theme.muted_text),
            ),
        ]));
        for body_line in wrap_body(
            &report.body,
            usize::from(popup_rect.width.saturating_sub(4)),
        ) {
            lines.push(Line::from(Span::styled(
                body_line,
                Style::default().fg(theme.primary_text),
            )));
        }
        lines.push(Line::from(""));
    }
    // Drop the trailing blank line from the last report.
    lines.pop();

    frame.render_widget(Clear, popup_rect);
    let block = Block::default()
        .title(Span::styled(
            format!(" {} ", row.name),
            Style::default().fg(theme.popup_title),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_unfocused));
    frame.render_widget(block, popup_rect);

    let inner = Rect {
        x: popup_rect.x + 1,
        y: popup_rect.y + 1,
        width: popup_rect.width.saturating_sub(2),
        height: popup_rect.height.saturating_sub(2),
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(lines).scroll((
            u16::try_from(reports.len().saturating_sub(MAX_REPORTS).saturating_mul(3))
                .unwrap_or(u16::MAX),
            0,
        )),
        inner,
    );
}

/// Computes the popup rect, sharing the task-list preview's geometry:
///
/// - **Right edge** touches the sidebar's left edge, not the frame's — the
///   popup describes a sidebar row, so it hangs off the sidebar.
/// - **Top edge** is the selected attendant's row. When the popup would not
///   fit below that row, it slides up to end at the status bar instead of
///   overflowing the frame.
/// - **Width** is 60% of the frame, floored at [`MIN_POPUP_WIDTH`], capped to
///   the space left of the sidebar.
/// - **Height** is the natural content height (borders + report lines),
///   capped to the room available, floored at [`MIN_POPUP_HEIGHT`].
///
/// Returns `None` when there is no room to draw (width or available height
/// below the floor).
fn popup_rect(
    frame_area: Rect,
    sidebar_rect: Rect,
    cursor_y: u16,
    report_count: usize,
) -> Option<Rect> {
    // Width is capped to the space LEFT of the sidebar, since the popup
    // hangs there rather than over the sidebar.
    let space_left_of_sidebar = sidebar_rect.x.saturating_sub(frame_area.x);
    let popup_width = (f32::from(frame_area.width) * 0.6).ceil() as u16;
    let popup_width = popup_width.max(MIN_POPUP_WIDTH).min(space_left_of_sidebar);
    if popup_width < MIN_POPUP_WIDTH {
        return None;
    }

    // The status bar owns the last two rows; the popup may not enter them.
    let bottom_bound = frame_area
        .y
        .saturating_add(frame_area.height.saturating_sub(2));

    // Hang below the attendant row when the popup fits there, so the box
    // always describes the row the cursor is on. Otherwise slide it up to
    // end at the status bar — never below the frame, never into the status
    // bar.
    let room_below = bottom_bound.saturating_sub(cursor_y);
    // Two lines per report (header + body) plus a spacer, plus 2 borders.
    let desired_height = u16::try_from(report_count.saturating_mul(3))
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let (popup_y, popup_height) = if room_below >= MIN_POPUP_HEIGHT {
        let height = desired_height.min(room_below).max(MIN_POPUP_HEIGHT);
        (cursor_y, height)
    } else {
        let max_height = bottom_bound.saturating_sub(frame_area.y);
        if max_height < MIN_POPUP_HEIGHT {
            return None;
        }
        let height = desired_height.min(max_height).max(MIN_POPUP_HEIGHT);
        (bottom_bound.saturating_sub(height), height)
    };

    // Right edge touches the sidebar's left edge.
    let popup_x = sidebar_rect.x.saturating_sub(popup_width);

    Some(Rect::new(popup_x, popup_y, popup_width, popup_height))
}

/// Wraps a report body to the popup's inner width, by grapheme clusters.
fn wrap_body(body: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![body.to_owned()];
    }
    let mut wrapped = Vec::new();
    for line in body.lines() {
        let mut current = String::new();
        let mut current_len = 0usize;
        for grapheme in line.graphemes(true) {
            let grapheme_len = grapheme.graphemes(true).count();
            if current_len + grapheme_len > width && !current.is_empty() {
                wrapped.push(std::mem::take(&mut current));
                current_len = 0;
            }
            current.push_str(grapheme);
            current_len += grapheme_len;
        }
        wrapped.push(current);
    }
    wrapped
}

#[cfg(test)]
mod tests {
    #![allow(
        unused_mut,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::uninlined_format_args,
        reason = "test code"
    )]
    use super::*;
    use jinn_kernel::common::app_state::AppState;
    use jinn_kernel::common::render_ctx::{RenderCtx, empty_config_layer};
    use jinn_session_state::ChatSessionState;

    /// A 120-wide frame with the sidebar in the rightmost 30 columns —
    /// the same shape the task-list preview tests use.
    fn frame_area() -> Rect {
        Rect::new(0, 0, 120, 40)
    }

    fn sidebar_rect() -> Rect {
        Rect::new(90, 0, 30, 40)
    }

    /// A state focused on the Attendants section with `count` attendants
    /// (each with one report) under the active parent, cursor on `selected`.
    fn state_focused_on_attendant(count: usize, selected: usize) -> AppState {
        let mut state = AppState::default_with_scope_focus();
        let parent = ChatSessionState::new();
        let parent_id = parent.session_id().clone();
        state.session.insert(parent);
        for i in 0..count {
            let mut attendant = {
                let parent = state.session.get(&parent_id).expect("parent").clone();
                ChatSessionState::new_attendant(&parent, true)
            };
            attendant.set_title(format!("reviewer {i}"));
            attendant.append_attendant_report(format!("finding {i}"));
            attendant.set_session_state(jinn_session_store_msg::SessionState::Loaded);
            state.session.insert(attendant);
        }
        state
            .frontend
            .scope_push(jinn_sidebar_msg::SidebarSectionId::Attendant.focus_scope());
        state.session.set_active(parent_id);
        let nth = jinn_attendant::section_rows::attendant_rows(&state)
            .into_iter()
            .nth(selected)
            .map(|row| row.session_id);
        state
            .frontend
            .update_sections(|s| s.attendant.selected_id = nth);
        state
    }

    /// The row carrying the popup's top-left corner glyph.
    fn popup_row(buffer: &ratatui::buffer::Buffer, frame: Rect) -> u16 {
        (frame.y..frame.height)
            .find(|&y| {
                (0..buffer.area().width)
                    .filter_map(|x| buffer.cell((x, y)))
                    .any(|cell| cell.symbol() == "\u{250c}")
            })
            .unwrap_or(frame.y)
    }

    /// Renders just the popup over `state` and returns the buffer plus the
    /// sidebar rect used.
    fn render_popup(
        state: &AppState,
        frame_area: Rect,
        sidebar_rect: Rect,
    ) -> (ratatui::buffer::Buffer, Rect) {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut terminal =
            Terminal::new(TestBackend::new(frame_area.width, frame_area.height)).expect("terminal");
        terminal
            .draw(|frame| {
                let slices = jinn_slices::Slices::new();
                let overlays = jinn_slices::OverlayViews::new();
                let ctx = RenderCtx::new_with_default_config(state, &slices, &overlays);
                render_attendant_reports_for_state(frame, sidebar_rect, frame_area, &ctx);
            })
            .expect("draw");
        (terminal.backend().buffer().clone(), sidebar_rect)
    }

    #[rstest::rstest]
    #[test]
    fn the_reports_popup_right_edge_meets_the_sidebar() {
        // Given a frame with the sidebar on the right and one attendant.
        let state = state_focused_on_attendant(1, 0);

        // When rendering the popup over that frame.
        let (buffer, sidebar) = render_popup(&state, frame_area(), sidebar_rect());

        // Then the popup's rightmost column is the one left of the sidebar —
        // find it by the popup's top-right corner on the top border row.
        let frame = frame_area();
        let row = popup_row(&buffer, frame);
        let popup_right = (frame.x..sidebar.x).rev().find(|&x| {
            buffer
                .cell((x, row))
                .is_some_and(|cell| cell.symbol() == "\u{2510}")
        });
        assert_eq!(
            popup_right,
            Some(sidebar.x.saturating_sub(1)),
            "popup right edge must touch the sidebar's left edge"
        );
    }

    #[rstest::rstest]
    #[test]
    fn the_reports_popup_hangs_from_the_selected_row() {
        // Given one attendant selected.
        let state = state_focused_on_attendant(1, 0);
        let frame = frame_area();
        let sidebar = sidebar_rect();
        let expected_top = crate::sections::layout::frame_row_of(
            sidebar,
            &state,
            empty_config_layer(),
            jinn_sidebar_msg::SidebarSectionId::Attendant,
            crate::sections::layout::cursor_row_in_section(
                &state,
                jinn_sidebar_msg::SidebarSectionId::Attendant,
            )
            .unwrap_or(0),
        );

        // When rendering the popup in a frame tall enough to fit below.
        let (buffer, _sidebar) = render_popup(&state, frame, sidebar);

        // Then the popup's top border is the attendant's own row.
        let top = (frame.y..frame.height)
            .find(|&y| {
                (0..sidebar.x)
                    .filter_map(|x| buffer.cell((x, y)))
                    .any(|cell| cell.symbol() == "\u{250c}")
            })
            .unwrap_or_else(|| panic!("no popup top border"));
        assert_eq!(top, expected_top, "popup hangs from the attendant row");
    }

    #[rstest::rstest]
    #[test]
    fn the_reports_popup_slides_up_when_the_row_is_low() {
        // Given one attendant selected with the sidebar anchored low in the
        // frame — the attendant's row sits below where the popup fits.
        let state = state_focused_on_attendant(1, 0);
        let frame = Rect::new(0, 0, 120, 12);
        let sidebar = Rect::new(90, 10, 30, 2);

        // When rendering the popup.
        let (buffer, _sidebar) = render_popup(&state, frame, sidebar);

        // Then the popup ends at the status bar's top edge rather than
        // overflowing the frame bottom.
        let bottom = (0..frame.height)
            .rev()
            .find(|&y| {
                (0..sidebar.x)
                    .filter_map(|x| buffer.cell((x, y)))
                    .any(|cell| cell.symbol() == "\u{2514}" || cell.symbol() == "\u{2518}")
            })
            .unwrap_or_else(|| panic!("no popup bottom border in a 12-row frame"));
        assert_eq!(
            bottom,
            frame.height.saturating_sub(3),
            "popup slides up to end above the status bar"
        );
    }

    #[rstest::rstest]
    #[test]
    fn the_reports_popup_width_is_capped_left_of_the_sidebar() {
        // Given a frame whose sidebar starts 35 columns in — less than the
        // 60% width the popup wants.
        let state = state_focused_on_attendant(1, 0);
        let frame = Rect::new(0, 0, 65, 40);
        let sidebar = Rect::new(35, 0, 30, 40);

        // When rendering the popup.
        let (buffer, sidebar) = render_popup(&state, frame, sidebar);

        // Then the popup starts at the frame's left edge — the width was
        // capped to the space left of the sidebar (top-left corner on x=0).
        let row = popup_row(&buffer, frame);
        let left = (0..sidebar.x).find(|&x| {
            buffer
                .cell((x, row))
                .is_some_and(|cell| cell.symbol() == "\u{250c}")
        });
        assert_eq!(left, Some(0), "popup width is capped to x=0");
    }

    #[rstest::rstest]
    #[test]
    fn the_reports_popup_anchors_to_the_second_attendant() {
        // Given two attendants with the cursor on the second.
        let state = state_focused_on_attendant(2, 1);
        let frame = frame_area();
        let sidebar = sidebar_rect();
        let expected_top = crate::sections::layout::frame_row_of(
            sidebar,
            &state,
            empty_config_layer(),
            jinn_sidebar_msg::SidebarSectionId::Attendant,
            crate::sections::layout::cursor_row_in_section(
                &state,
                jinn_sidebar_msg::SidebarSectionId::Attendant,
            )
            .unwrap_or(0),
        );

        // When rendering the popup.
        let (buffer, _sidebar) = render_popup(&state, frame, sidebar);

        // Then the popup hangs from the *second* attendant's row — two rows
        // below the first attendant's.
        let top = (frame.y..frame.height)
            .find(|&y| {
                (0..sidebar.x)
                    .filter_map(|x| buffer.cell((x, y)))
                    .any(|cell| cell.symbol() == "\u{250c}")
            })
            .unwrap_or_else(|| panic!("no popup top border"));
        assert_eq!(
            top, expected_top,
            "popup anchors to the second attendant, not the section header"
        );
    }
}
