//! [`TaskListSection`] — the task list sidebar section.
//!
//! Render-only section that displays the phased task list for the active session.
//! Navigation is always `Exhausted` — the cursor skips over this section.
//! The section is hidden when the task list is empty.

use crate::common::app_state::AppState;
use crate::feat::task_list::{TaskList, TaskStatus};
use crate::feat::ui::sidebar::section_trait::{
    EnterFrom, SectionNavResult, SidebarIntent, SidebarSection, SidebarSectionId,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// The task list sidebar section.
///
/// Renders phases and tasks from the active session's task list.
/// This section is non-interactive — the cursor always skips over it.
#[derive(Debug)]
pub struct TaskListSection;

/// Navigate within the task list section.
///
/// Always returns `Exhausted` — the section is non-interactive.
pub fn navigate(_intent: &SidebarIntent, _state: &mut AppState) -> SectionNavResult {
    SectionNavResult::Exhausted
}

/// Place the cursor on this section (no-op).
pub fn receive_cursor(_state: &mut AppState, _enter_from: EnterFrom) {
    // No-op: non-interactive section.
}

impl SidebarSection for TaskListSection {
    fn id(&self) -> SidebarSectionId {
        SidebarSectionId::TaskList
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        let list = state.active_session().task_list();
        if list.is_empty() {
            return;
        }

        let lines = build_render_lines(list, state);
        let widget = Paragraph::new(lines);
        frame.render_widget(widget, area);
    }

    fn content_height(&self, state: &AppState) -> u16 {
        let list = state.active_session().task_list();
        if list.is_empty() {
            return 0;
        }
        compute_height(list)
    }
}

/// Builds the render lines for a task list.
fn build_render_lines(list: &TaskList, state: &AppState) -> Vec<Line<'static>> {
    let theme = &state.frontend.theme;
    let mut lines = Vec::new();

    // Header.
    let phase_count = list.phases().len();
    lines.push(Line::from(vec![Span::styled(
        format!(
            " Task List \u{2014} {} phase{}",
            phase_count,
            if phase_count == 1 { "" } else { "s" }
        ),
        Style::default()
            .fg(theme.primary_text)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    for phase in list.phases() {
        // Phase header.
        lines.push(Line::from(vec![Span::styled(
            format!("  {}", phase.description()),
            Style::default()
                .fg(theme.muted_text)
                .add_modifier(Modifier::BOLD),
        )]));

        if phase.is_empty() {
            lines.push(Line::from(Span::styled(
                "    (no tasks)",
                Style::default().fg(theme.muted_text),
            )));
        } else {
            for task in phase.tasks() {
                let indicator = match task.status() {
                    TaskStatus::Pending => "\u{25CB} ",   // ○
                    TaskStatus::Completed => "\u{2713} ", // ✓
                };
                let style = if task.status() == TaskStatus::Completed {
                    Style::default().fg(theme.muted_text)
                } else {
                    Style::default().fg(theme.primary_text)
                };
                lines.push(Line::from(Span::styled(
                    format!("    {}{}", indicator, task.description()),
                    style,
                )));
            }
        }

        // Blank line between phases.
        lines.push(Line::from(""));
    }

    // Remove trailing blank.
    if lines.last() == Some(&Line::from("")) {
        lines.pop();
    }

    lines
}

/// Computes the content height for a non-empty task list.
fn compute_height(list: &TaskList) -> u16 {
    let mut height: usize = 0;

    // Header + blank.
    height += 2;

    for phase in list.phases() {
        // Phase header.
        height += 1;
        if phase.is_empty() {
            height += 1; // "(no tasks)"
        } else {
            height += phase.tasks().len();
        }
        // Blank line between phases.
        height += 1;
    }

    // Subtract trailing blank + add trailing gap.
    height.saturating_sub(1) as u16 + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::app_state::AppState;
    use crate::feat::task_list::TaskPosition;

    fn setup_with_tasks() -> AppState {
        let mut app = AppState::default();
        let session = app.session.active_session_mut();
        let pid = session.task_list_mut().add_phase("Research");
        session
            .task_list_mut()
            .add_task(&pid, "Read docs", TaskPosition::End)
            .unwrap();
        session
            .task_list_mut()
            .add_task(&pid, "Call API", TaskPosition::End)
            .unwrap();
        let pid2 = session.task_list_mut().add_phase("Build");
        session
            .task_list_mut()
            .add_task(&pid2, "Write code", TaskPosition::End)
            .unwrap();
        app
    }

    #[test]
    fn content_height_is_zero_when_empty() {
        let app = AppState::default();
        let section = TaskListSection;
        assert_eq!(section.content_height(&app), 0);
    }

    #[test]
    fn content_height_is_nonzero_when_has_phases() {
        let app = setup_with_tasks();
        let section = TaskListSection;
        let height = section.content_height(&app);
        assert!(height > 0, "expected non-zero height, got {}", height);
    }

    #[test]
    fn navigate_always_exhausted() {
        let mut app = AppState::default();
        let result = navigate(&SidebarIntent::MoveDown, &mut app);
        assert_eq!(result, SectionNavResult::Exhausted);
    }

    #[test]
    fn receive_cursor_is_noop() {
        let mut app = AppState::default();
        // Should not panic.
        receive_cursor(&mut app, EnterFrom::Top);
    }

    #[test]
    fn build_render_lines_shows_phases() {
        let app = setup_with_tasks();
        let list = app.session.active_session().task_list().clone();
        let lines = build_render_lines(&list, &app);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
            .collect();
        let combined = text.join("\n");
        assert!(
            combined.contains("Research"),
            "should contain phase: Research"
        );
        assert!(combined.contains("Build"), "should contain phase: Build");
        assert!(
            combined.contains("Read docs"),
            "should contain task: Read docs"
        );
        assert!(
            combined.contains("Write code"),
            "should contain task: Write code"
        );
    }

    #[test]
    fn build_render_lines_shows_pending_indicator() {
        let app = setup_with_tasks();
        let list = app.session.active_session().task_list().clone();
        let lines = build_render_lines(&list, &app);
        let combined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect();
        assert!(
            combined.contains("\u{25CB}"),
            "should contain pending indicator ○"
        );
    }

    #[test]
    fn build_render_lines_shows_completed_indicator() {
        let mut app = AppState::default();
        let session = app.session.active_session_mut();
        let pid = session.task_list_mut().add_phase("Build");
        let tid = session
            .task_list_mut()
            .add_task(&pid, "Write code", TaskPosition::End)
            .unwrap();
        session.task_list_mut().complete_task(&tid).unwrap();
        let list = session.task_list().clone();
        let lines = build_render_lines(&list, &app);
        let combined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect();
        assert!(
            combined.contains("\u{2713}"),
            "should contain completed indicator ✓"
        );
    }

    #[test]
    fn id_returns_task_list() {
        let section = TaskListSection;
        assert_eq!(section.id(), SidebarSectionId::TaskList);
    }
}
