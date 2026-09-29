//! The Attendants sidebar section — sessions watching another session.
//!
//! One row per loaded attendant session: line 1 is the session's name (its
//! identity — an attendant has no separate label), line 2 is the body of its
//! most recent `report` call. A report that predates the parent's latest
//! activity renders muted (stale); an attendant that has never reported shows
//! a distinct marker instead of an empty second line.

use crate::sections::section_trait::{EnterFrom, SectionNavResult, SidebarIntent};
use jinn_attendant::section_rows::attendant_rows;
use jinn_kernel::AppState;
use jinn_sidebar_msg::AttendantSectionState;

/// The marker an attendant that has never reported shows on line 2.
pub(crate) const NEVER_REPORTED_MARKER: &str = jinn_attendant_msg::AttendantReport::EMPTY_MARKER;

/// Marks an attendant in seed mode, matching the sessions section's glyph.
const ATTENDANT_PAUSED_SYMBOL: &str = "⏸ ";

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
    let row = attendant_rows(state)
        .len()
        .checked_sub(1)
        .map_or(0, |last| row_index_of(index.min(last)));
    Some(2u16.saturating_add(row))
}

/// The section-relative row of one attendant's first line.
///
/// Each attendant renders two lines (name + report preview), so the
/// *n*-th attendant's name line is section row `2n` below the rows block.
/// [`cursor_row`] adds the header and separator offset.
fn row_index_of(index: usize) -> u16 {
    let idx = u16::try_from(index).unwrap_or(u16::MAX);
    idx.saturating_mul(2)
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
            let indicator =
                crate::sections::session_row_style::chip_span(is_selected, sidebar_focused, theme);
            // The name carries no style of its own when selected: a span with
            // a hard foreground would defeat the band, and a selected row is
            // the band. Unselected, the pink marks the row as an attendant.
            let name_style = if is_selected {
                Style::default()
            } else {
                Style::default().fg(theme.attendant_fg)
            };
            // Ahead of the name, where the sessions section puts the same
            // marker: the eye reads the marker before the title. And beside
            // the name rather than inside it — the name is what a rename
            // replaces, so the marker must stay out of its reach. Like every
            // state color it yields to the selection band when selected.
            let paused = if !row.is_paused {
                Span::raw("")
            } else if is_selected {
                Span::raw(ATTENDANT_PAUSED_SYMBOL)
            } else {
                Span::styled(
                    ATTENDANT_PAUSED_SYMBOL,
                    Style::default().fg(theme.attendant_paused),
                )
            };
            let content_width =
                2 + if row.is_paused { 2 } else { 0 } + 1 + row.name.chars().count();
            let mut row_spans = vec![
                indicator,
                crate::sections::session_row_style::chip_gap(),
                paused,
                Span::styled(format!(" {}", row.name), name_style),
            ];
            if is_selected {
                // The pad carries the band to the row's last cell —
                // `Paragraph` does not extend a line's style past the last
                // grapheme.
                row_spans.push(crate::sections::session_row_style::band_pad(
                    content_width,
                    usize::from(area.width),
                    theme,
                ));
            }
            let row_line = Line::from(row_spans);
            let row_line = if is_selected {
                row_line.style(crate::sections::session_row_style::selected_row_style(
                    theme,
                ))
            } else {
                row_line
            };
            lines.push(row_line);

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

    use super::{AttendantsSection, has_content};
    use crate::sections::section_trait::SidebarSection;
    use jinn_kernel::AppState;

    /// State whose active session is `active`, holding `attendant` as its
    /// child, plus an unrelated attendant under some other parent.
    fn state_with(
        active: jinn_session_state::ChatSessionState,
        attendant: jinn_session_state::ChatSessionState,
    ) -> AppState {
        let mut state = AppState::default_with_scope_focus();
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

    /// Renders the section over `state` and returns the buffer, so a test
    /// can read the cells the user actually sees.
    fn render_section(state: &AppState) -> ratatui::buffer::Buffer {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut terminal = Terminal::new(TestBackend::new(40, 20)).expect("terminal");
        terminal
            .draw(|frame| {
                let slices = jinn_slices::Slices::new();
                let overlays = jinn_slices::OverlayViews::new();
                let ctx = jinn_kernel::common::render_ctx::RenderCtx::new_with_default_config(
                    state, &slices, &overlays,
                );
                AttendantsSection.render(frame, ratatui::layout::Rect::new(0, 0, 40, 20), 0, &ctx);
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    /// The rendered screen as text, one string per row.
    fn screen_text(buffer: &ratatui::buffer::Buffer) -> String {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// A parent with one paused attendant of the given configuration.
    fn state_with_paused_attendant(
        activation: jinn_attendant_msg::AttendantActivation,
        trigger: jinn_attendant_msg::AttendantTrigger,
    ) -> AppState {
        let parent = user_session();
        let mut attendant = attendant_of(&parent);
        attendant.set_title("reviewer".to_owned());
        attendant.set_attendant_activation(activation);
        attendant.set_attendant_trigger(trigger);
        state_with(parent, attendant)
    }

    #[rstest::rstest]
    fn a_paused_attendant_row_shows_the_marker_before_the_title() {
        // Given a paused attendant under an active parent.
        let state = state_with_paused_attendant(
            jinn_attendant_msg::AttendantActivation::Seed,
            jinn_attendant_msg::AttendantTrigger::Manual,
        );

        // When the section is rendered.
        let buffer = render_section(&state);

        // Then the marker leads the name, matching the sessions section —
        // the eye reads the marker before the title.
        let text = screen_text(&buffer);
        let line = text
            .lines()
            .find(|line| line.contains('\u{23F8}'))
            .unwrap_or_else(|| panic!("no pause marker rendered: {text:?}"));
        let marker = line.find('\u{23F8}').expect("marker column");
        let name = line.find("reviewer").expect("attendant name");
        assert!(marker < name, "the marker must precede the name: {line:?}");
    }

    #[rstest::rstest]
    fn the_pause_marker_sits_outside_the_title() {
        // Given a paused attendant.
        let state = state_with_paused_attendant(
            jinn_attendant_msg::AttendantActivation::Seed,
            jinn_attendant_msg::AttendantTrigger::Manual,
        );

        // When the section is rendered.
        let buffer = render_section(&state);

        // Then the name's own cells carry the name alone — no marker glued
        // to it. The name is what a rename replaces, so the marker must
        // live in a span the rename cannot reach.
        let text = screen_text(&buffer);
        let line = text
            .lines()
            .find(|line| line.contains("reviewer"))
            .unwrap_or_else(|| panic!("no attendant row rendered: {text:?}"));
        let name = line.find("reviewer").expect("attendant name");
        let marked = line
            .chars()
            .skip(name)
            .take("reviewer".len())
            .any(|c| c == '\u{23F8}');
        assert!(
            !marked,
            "the name's own cells must be the name alone: {line:?}"
        );
    }

    #[rstest::rstest]
    fn a_selected_attendant_row_takes_the_selection_band_not_pink() {
        // Given an attendant with the section's cursor on it.
        let parent = user_session();
        let mut attendant = attendant_of(&parent);
        attendant.set_title("reviewer".to_owned());
        let state = state_with(parent, attendant);
        state
            .frontend
            .scope_push(jinn_sidebar_msg::SidebarSectionId::Attendant.focus_scope());
        state
            .frontend
            .update_sections(|s| s.attendant.selected_index = Some(0));
        let theme = state.frontend.theme.clone();

        // When the section is rendered into a buffer.
        let buffer = render_section_wide(&state, 60);

        // Then the attendant's row carries the selection band...
        let band_y = (0..buffer.area().height)
            .find(|&y| {
                (0..buffer.area().width).any(|x| {
                    buffer
                        .cell((x, y))
                        .is_some_and(|cell| cell.bg == theme.selection_bg)
                })
            })
            .unwrap_or_else(|| panic!("no selection band rendered"));
        // ...reaching the row's last cell — the band is full-width.
        let last_banded_x = (0..buffer.area().width)
            .filter(|&x| {
                buffer
                    .cell((x, band_y))
                    .is_some_and(|cell| cell.bg == theme.selection_bg)
            })
            .max();
        assert_eq!(
            last_banded_x,
            Some(buffer.area().width.saturating_sub(1)),
            "the band must reach the row's last cell"
        );
        // And the name is band text, not the attendant pink — selection
        // overrides the state color.
        let text: String = (0..buffer.area().width)
            .filter_map(|x| buffer.cell((x, band_y)).map(ratatui::buffer::Cell::symbol))
            .collect();
        let name_at = text.find("reviewer").expect("attendant name visible");
        let name_cell = buffer
            .cell((u16::try_from(name_at).unwrap_or(0), band_y))
            .expect("name cell");
        assert_eq!(name_cell.fg, theme.gutter_bg);
        assert_ne!(name_cell.fg, theme.attendant_fg);
        // And the chip cell stays dark against the band.
        let chip = buffer.cell((0, band_y)).expect("chip cell");
        assert_eq!(chip.bg, theme.gutter_bg);
    }

    /// Renders the section into a buffer of the given width.
    fn render_section_wide(state: &AppState, width: u16) -> ratatui::buffer::Buffer {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let height = 20u16;
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| {
                let slices = jinn_slices::Slices::new();
                let overlays = jinn_slices::OverlayViews::new();
                let ctx = jinn_kernel::common::render_ctx::RenderCtx::new_with_default_config(
                    state, &slices, &overlays,
                );
                AttendantsSection.render(
                    frame,
                    ratatui::layout::Rect::new(0, 0, width, height),
                    0,
                    &ctx,
                );
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }
}
