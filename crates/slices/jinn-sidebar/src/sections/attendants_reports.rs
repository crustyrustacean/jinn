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
/// Rows between the popup and the attendant row it describes.
const POPUP_GAP: u16 = 2;
/// Minimum popup width in cells; below this the frame is too narrow and the
/// popup is skipped rather than drawn unreadably small.
const MIN_POPUP_WIDTH: u16 = 30;

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
    let Some(idx) = state
        .frontend
        .with_sections(|s| s.attendant.selected_index, || None)
    else {
        return;
    };
    let rows = jinn_attendant::section_rows::attendant_rows(state);
    let Some(row) = rows.get(idx) else {
        return;
    };
    let Some(session) = state.session.get(&row.session_id) else {
        return;
    };
    let reports = session.attendant_reports();
    let theme = &state.frontend.theme;

    // Anchor the popup to the attendant's own row through the same document
    // layout the sidebar renders with, so it stays attached while scrolling.
    let cursor_y = crate::sections::layout::frame_row_of(
        sidebar_rect,
        state,
        ctx.config(),
        jinn_sidebar_msg::SidebarSectionId::Attendant,
        u16::try_from(idx).unwrap_or(u16::MAX),
    );

    let shown = reports
        .len()
        .saturating_sub(reports.len().saturating_sub(MAX_REPORTS));
    let popup_rect = popup_rect(frame_area, cursor_y, shown);

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

/// Computes the popup rect: right-aligned with the frame edge, bottom edge
/// one gap above the attendant's row. Height is a function of the reports
/// shown, capped to the space above the row (minimum 5 rows so the empty
/// state stays visible).
fn popup_rect(frame_area: Rect, cursor_y: u16, report_count: usize) -> Rect {
    let popup_width = ((f32::from(frame_area.width) * 0.6).ceil() as u16)
        .max(MIN_POPUP_WIDTH)
        .min(frame_area.width);

    // Two lines per report (header + body) plus a spacer, plus 2 borders.
    let desired_height = u16::try_from(report_count.saturating_mul(3))
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let max_height = cursor_y
        .saturating_sub(frame_area.y)
        .saturating_sub(POPUP_GAP);
    let popup_height = desired_height.min(max_height).max(5);

    let popup_x = frame_area.x + frame_area.width.saturating_sub(popup_width);
    let popup_y = cursor_y
        .saturating_sub(popup_height)
        .saturating_sub(POPUP_GAP);

    Rect::new(popup_x, popup_y, popup_width, popup_height)
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
