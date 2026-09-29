//! The Attendants sidebar section — sessions watching another session.
//!
//! One row per loaded attendant session: line 1 is the session's name (its
//! identity — an attendant has no separate label), line 2 is the body of its
//! most recent `report` call. A report that predates the parent's latest
//! activity renders muted (stale); an attendant that has never reported shows
//! a distinct marker instead of an empty second line.

use crate::sections::section_trait::{EnterFrom, SectionNavResult, SidebarIntent};
use jinn_attendant::section_rows::{AttendantRow, attendant_rows};
use jinn_kernel::AppState;
use jinn_sidebar_msg::AttendantSectionState;

/// The marker an attendant that has never reported shows on line 2.
pub(crate) const NEVER_REPORTED_MARKER: &str = jinn_attendant_msg::AttendantReport::EMPTY_MARKER;

/// Cursor indicator, matching the other sections' glyph.
const SELECTED_INDICATOR: &str = "\u{2588}";

/// The unselected cursor column (blank, keeps alignment).
const UNSELECTED_BORDER: &str = " ";

/// Whether the section has any rows — an empty section collapses.
pub(crate) fn has_content(state: &AppState) -> bool {
    !attendant_rows(state).is_empty()
}

/// The section's rendered row count (header + separator + rows + gap).
pub(crate) fn rows(state: &AppState) -> u16 {
    if !has_content(state) {
        return 0;
    }
    // Header + blank separator + two lines per attendant + a trailing blank
    // so the section does not run straight into the one below it. The
    // render emits the same lines, and `content_height` defers here, so the
    // three cannot disagree about where the next section starts.
    let count = u16::try_from(attendant_rows(state).len()).unwrap_or(u16::MAX);
    count.saturating_mul(2).saturating_add(3)
}

/// The cursor row relative to the section's first rendered row.
pub(crate) fn cursor_row(state: &AppState) -> Option<u16> {
    let index = selected_index(state)?;
    let row = attendant_rows(state).get(index).map_or(0, row_index_of);
    Some(2u16.saturating_add(row))
}

/// The section-relative row of one attendant's first line.
fn row_index_of(_row: &AttendantRow) -> u16 {
    0
}

fn selected_index(state: &AppState) -> Option<usize> {
    state.frontend.with_sections(
        |sections: &jinn_sidebar_msg::SidebarSections| sections.attendant.selected_index,
        || None,
    )
}

/// Place the cursor on this section from a given direction.
pub(crate) fn receive_cursor(state: &mut AppState, enter_from: EnterFrom) {
    let count = attendant_rows(state).len();
    if count == 0 {
        return;
    }
    let index = match enter_from {
        EnterFrom::Top => 0,
        EnterFrom::Bottom => count - 1,
    };
    state
        .frontend
        .update_sections(|s: &mut jinn_sidebar_msg::SidebarSections| {
            s.attendant = AttendantSectionState {
                selected_index: Some(index),
            };
        });
}

/// Navigate the section's cursor, reporting exhaustion at the edges.
pub(crate) fn navigate(intent: &SidebarIntent, state: &mut AppState) -> SectionNavResult {
    let count = attendant_rows(state).len();
    if count == 0 {
        return SectionNavResult::Exhausted;
    }
    let max_index = count - 1;
    let current = selected_index(state).unwrap_or(0);
    match intent {
        SidebarIntent::MoveDown => {
            if current >= max_index {
                SectionNavResult::Exhausted
            } else {
                state
                    .frontend
                    .update_sections(|s| s.attendant.selected_index = Some(current + 1));
                SectionNavResult::Moved
            }
        }
        SidebarIntent::MoveUp => {
            if current == 0 {
                SectionNavResult::Exhausted
            } else {
                state
                    .frontend
                    .update_sections(|s| s.attendant.selected_index = Some(current - 1));
                SectionNavResult::Moved
            }
        }
        SidebarIntent::Action(_) => SectionNavResult::Moved,
    }
}

/// The Attendants sidebar section.
///
/// Renders a header, then two lines per attendant: the session name (the
/// attendant's identity — there is no separate label), then the latest
/// report body, muted when stale, or a never-reported marker.
#[derive(Debug)]
pub struct AttendantsSection;

impl crate::sections::section_trait::SidebarSection for AttendantsSection {
    fn id(&self) -> jinn_sidebar_msg::SidebarSectionId {
        jinn_sidebar_msg::SidebarSectionId::Attendant
    }

    fn render(
        &mut self,
        frame: &mut ratatui::Frame<'_>,
        area: ratatui::layout::Rect,
        skip_rows: u16,
        ctx: &dyn jinn_slices::DrawContext<jinn_kernel::common::app_state::AppState>,
    ) {
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, Paragraph};

        let state = ctx.state();
        let sidebar_focused = state.frontend.is_sidebar();
        let section_focused = sidebar_focused
            && state.frontend.sidebar_section()
                == Some(jinn_sidebar_msg::SidebarSectionId::Attendant);
        let cursor = selected_index(state);
        let theme = &state.frontend.theme;

        let indicator_color = if sidebar_focused {
            theme.focus_accent
        } else {
            theme.border_unfocused
        };

        let mut lines = Vec::new();
        lines.push(Line::from(vec![Span::styled(
            " Attendants",
            Style::default()
                .fg(theme.primary_text)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));

        for (index, row) in attendant_rows(state).into_iter().enumerate() {
            let is_selected = section_focused && cursor == Some(index);
            let indicator = if is_selected {
                Span::styled(SELECTED_INDICATOR, Style::default().fg(indicator_color))
            } else {
                Span::raw(UNSELECTED_BORDER)
            };
            let name_style = if is_selected {
                Style::default()
                    .fg(theme.attendant_fg)
                    .add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(theme.attendant_fg)
            };
            lines.push(Line::from(vec![
                indicator,
                Span::styled(format!(" {}", row.name), name_style),
            ]));

            let report_line = match &row.latest_report {
                Some(body) => {
                    let style = if row.is_stale {
                        Style::default().fg(theme.muted_text)
                    } else {
                        Style::default().fg(theme.primary_text)
                    };
                    Span::styled(truncate_report(body), style)
                }
                None => Span::styled(NEVER_REPORTED_MARKER, Style::default().fg(theme.dormant_fg)),
            };
            lines.push(Line::from(vec![Span::raw("   "), report_line]));
        }

        // Trailing gap — the blank line that keeps the last report off the
        // next section's header. Counted by `rows`, like every sibling
        // section's gap.
        lines.push(Line::from(""));

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(ratatui::widgets::Borders::NONE))
            .scroll((skip_rows, 0));
        frame.render_widget(widget, area);
    }

    fn content_height(
        &mut self,
        ctx: &dyn jinn_slices::DrawContext<jinn_kernel::common::app_state::AppState>,
    ) -> u16 {
        // Defers to `rows` so the height the layout reserves and the lines
        // the render draws stay one number, gap included.
        rows(ctx.state())
    }
}

/// Truncates a report body to one display line.
fn truncate_report(body: &str) -> String {
    const MAX: usize = 48;
    let first_line = body.lines().next().unwrap_or("");
    if first_line.chars().count() <= MAX {
        return first_line.to_owned();
    }
    let mut cut: String = first_line.chars().take(MAX.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, reason = "test code")]

    use super::has_content;
    use jinn_kernel::AppState;

    /// State whose active session is `active`, holding `attendant` as its
    /// child, plus an unrelated attendant under some other parent.
    fn state_with(
        active: jinn_session_state::ChatSessionState,
        attendant: jinn_session_state::ChatSessionState,
    ) -> AppState {
        let mut state = AppState::default();
        let active_id = active.session_id().clone();
        state.session.insert(active);
        state.session.insert(attendant);
        state.session.set_active(active_id);
        state
    }

    /// A plain user session.
    fn user_session() -> jinn_session_state::ChatSessionState {
        jinn_session_state::ChatSessionState::new()
    }

    /// An attendant of `parent`.
    fn attendant_of(
        parent: &jinn_session_state::ChatSessionState,
    ) -> jinn_session_state::ChatSessionState {
        jinn_session_state::ChatSessionState::new_attendant(parent, true)
    }

    #[rstest::rstest]
    fn the_section_shows_on_a_parent_that_has_an_attendant() {
        // Given a parent session with one attendant, and the parent active.
        let parent = user_session();
        let state = state_with(parent.clone(), attendant_of(&parent));

        // When the section asks whether it has content.
        let content = has_content(&state);

        // Then it does — this is the screen the user is on.
        assert!(content);
    }

    #[rstest::rstest]
    fn the_section_shows_from_inside_an_attendant() {
        // Given an attendant with a sibling under the same parent, and the
        // first attendant active.
        let parent = user_session();
        let active = attendant_of(&parent);
        let mut state = state_with(active.clone(), active.clone());
        let sibling = attendant_of(&parent);
        let sibling_id = sibling.session_id().clone();
        state.session.insert(sibling);
        state.session.set_active(active.session_id().clone());

        // When the section asks whether it has content.
        let content = has_content(&state);

        // Then it does. An attendant is read in its parent's context, so its
        // siblings — the other reports on the same question — are what the
        // user is looking at.
        assert!(content);
        // And the sibling is the row that shows.
        let rows = jinn_attendant::section_rows::attendant_rows(&state);
        assert_eq!(rows.len(), 2, "both siblings belong to this context");
        assert!(rows.iter().any(|row| row.session_id == sibling_id));
    }

    #[rstest::rstest]
    fn the_section_hides_on_a_session_that_has_no_attendants() {
        // Given a bystander session with no attendant of its own, sitting
        // alongside another parent that does have one.
        let bystander = user_session();
        let elsewhere = user_session();
        let other_attendant = attendant_of(&elsewhere);
        let other_id = other_attendant.session_id().clone();
        let mut state = state_with(bystander.clone(), other_attendant);
        state.session.set_active(bystander.session_id().clone());

        // When the section asks whether it has content.
        let content = has_content(&state);

        // Then it does not. The other parent's attendant is loaded and
        // visible in the store, but it belongs to a different context, and
        // listing it here would answer a question the user did not ask.
        assert!(!content);
        // And no row leaks through.
        let rows = jinn_attendant::section_rows::attendant_rows(&state);
        assert!(rows.is_empty(), "leaked the attendant of another parent");
        assert!(
            !rows.iter().any(|row| row.session_id == other_id),
            "another parent's attendant must not appear here"
        );
    }
}
