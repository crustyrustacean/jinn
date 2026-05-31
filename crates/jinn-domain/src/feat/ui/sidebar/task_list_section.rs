//! [`TaskListSection`] - the task list sidebar section.
//!
//! Render-only section that displays the phased task list for the active session.
//! Navigation is always `Exhausted` - the cursor skips over this section.
//! The section is hidden when the task list is empty.

use std::borrow::Cow;

use crate::common::app_state::AppState;
use crate::feat::todo_list::{TaskList, TaskStatus};
use crate::feat::ui::sidebar::section_trait::{
    EnterFrom, SectionNavResult, SidebarIntent, SidebarSection, SidebarSectionId,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use textwrap::Options;

/// The task list sidebar section.
///
/// Renders phases and tasks from the active session's task list.
/// This section is non-interactive - the cursor always skips over it.
#[derive(Debug)]
pub struct TaskListSection;

/// Navigate within the task list section.
///
/// Always returns `Exhausted` - the section is non-interactive.
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
        compute_height(list, state.frontend.sidebar_width)
    }
}

/// Indent for phase descriptions (2 spaces).
const PHASE_INDENT: usize = 2;

/// Indent for task descriptions (4 spaces + 1 indicator + 1 space = 6 columns).
const TASK_INDENT: usize = 6;

/// Word-wraps a description string to the given available width.
///
/// Returns a single-element vec when `available_width` is too small to wrap.
fn wrap_description(text: &str, available_width: usize) -> Vec<String> {
    if available_width < 2 {
        return vec![text.to_owned()];
    }
    textwrap::wrap(text, Options::new(available_width))
        .into_iter()
        .map(Cow::into_owned)
        .collect()
}

/// Builds the render lines for a task list.
fn build_render_lines(list: &TaskList, state: &AppState) -> Vec<Line<'static>> {
    let theme = &state.frontend.theme;
    let sidebar_width = state.frontend.sidebar_width as usize;
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
        // Phase header - word-wrapped.
        let phase_width = sidebar_width.saturating_sub(PHASE_INDENT);
        let phase_style = Style::default()
            .fg(theme.muted_text)
            .add_modifier(Modifier::BOLD);
        let wrapped = wrap_description(phase.description(), phase_width);
        for segment in &wrapped {
            lines.push(Line::from(Span::styled(
                format!("  {segment}"),
                phase_style,
            )));
        }

        if phase.is_empty() {
            lines.push(Line::from(Span::styled(
                "    (no tasks)",
                Style::default().fg(theme.muted_text),
            )));
        } else {
            let task_width = sidebar_width.saturating_sub(TASK_INDENT);
            for task in phase.tasks() {
                let indicator = match task.status() {
                    TaskStatus::Pending => "\u{25CB} ",    // \u{25CB}  ○
                    TaskStatus::Completed => "\u{2713} ",  // \u{2713}  ✓
                    TaskStatus::Cancelled => {
                        // Cancelled tasks are hidden from sidebar.
                        continue;
                    }
                    TaskStatus::Postponed => "\u{25BC} ",   // \u{25BC}  ▼
                };
                let style = match task.status() {
                    TaskStatus::Pending => Style::default().fg(theme.primary_text),
                    TaskStatus::Cancelled => {
                        // Cancelled tasks are hidden (continued above), but rust needs this arm
                        Style::default().fg(theme.muted_text)
                    }
                    TaskStatus::Completed | TaskStatus::Postponed => {
                        Style::default().fg(theme.muted_text)
                    }
                };
                let wrapped = wrap_description(task.description(), task_width);
                for (i, segment) in wrapped.iter().enumerate() {
                    if i == 0 {
                        lines.push(Line::from(Span::styled(
                            format!("    {indicator}{segment}"),
                            style,
                        )));
                    } else {
                        lines.push(Line::from(Span::styled(
                            format!("      {segment}"),
                            style,
                        )));
                    }
                }
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
fn compute_height(list: &TaskList, sidebar_width: u16) -> u16 {
    let sidebar_width = sidebar_width as usize;
    let mut height: usize = 0;

    // Header + blank.
    height += 2;

    for phase in list.phases() {
        // Phase header - count wrapped lines.
        let phase_width = sidebar_width.saturating_sub(PHASE_INDENT);
        height += wrap_description(phase.description(), phase_width).len();

        if phase.is_empty() {
            height += 1; // "(no tasks)"
        } else {
            let task_width = sidebar_width.saturating_sub(TASK_INDENT);
            for task in phase.tasks() {
                height += wrap_description(task.description(), task_width).len();
            }
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
    use crate::feat::todo_list::TaskPosition;

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
        assert!(height > 0, "expected non-zero height, got {height}");
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

    #[test]
    fn build_render_lines_shows_postponed_indicator() {
        let mut app = AppState::default();
        let session = app.session.active_session_mut();
        let p1 = session.task_list_mut().add_phase("Research");
        let t1 = session
            .task_list_mut()
            .add_task(&p1, "Read docs", TaskPosition::End)
            .unwrap();
        let p2 = session.task_list_mut().add_phase("Build");
        let t2 = session
            .task_list_mut()
            .add_task(&p2, "Write code", TaskPosition::End)
            .unwrap();

        // Postpone t1 (Read docs) to after t2.
        session
            .task_list_mut()
            .postpone_task(&t1, crate::feat::todo_list::TaskPosition::After(t2))
            .unwrap();

        let list = session.task_list().clone();
        let lines = build_render_lines(&list, &app);
        let combined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect();

        // Sidebar should show the postponed task with ▼ indicator.
        assert!(
            combined.contains("\u{25BC}"),
            "should contain postponed indicator ▼"
        );
        // Sidebar should also show pending tasks.
        assert!(
            combined.contains("\u{25CB}"),
            "should contain pending indicator ○"
        );
    }

    // --- wrap_description tests ---

    #[test]
    fn wrap_description_short_text_no_wrap() {
        // Given text shorter than width.
        let result = wrap_description("hello", 20);

        // Then a single element is returned.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "hello");
    }

    #[test]
    fn wrap_description_long_text_wraps() {
        // Given text much longer than width.
        let text = "This is a very long description that should wrap";

        // When wrapping at width 15.
        let result = wrap_description(text, 15);

        // Then multiple lines are produced.
        assert!(result.len() > 1, "expected wrapping, got {result:?}");
        // And the joined text preserves all words.
        let joined = result.join(" ");
        assert!(joined.contains("very long"));
    }

    #[test]
    fn wrap_description_zero_width_returns_original() {
        // Given text and width 0.
        let result = wrap_description("hello", 0);

        // Then the original text is returned as a single element.
        assert_eq!(result, vec!["hello".to_owned()]);
    }

    #[test]
    fn wrap_description_empty_text_returns_single_empty() {
        // Given empty text.
        let result = wrap_description("", 20);

        // Then a single empty string is returned.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "");
    }

    // --- Word-wrap integration tests ---

    #[test]
    fn build_render_lines_long_task_wraps() {
        // Given a task with a long description and narrow sidebar.
        let mut app = AppState::default();
        app.frontend.sidebar_width = 20;
        let session = app.session.active_session_mut();
        let pid = session.task_list_mut().add_phase("Build");
        session
            .task_list_mut()
            .add_task(
                &pid,
                "This is a very long task description that should wrap",
                TaskPosition::End,
            )
            .unwrap();
        let list = session.task_list().clone();

        // When rendering.
        let lines = build_render_lines(&list, &app);

        // Then the full description text is present (not clipped).
        let combined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect();
        assert!(
            combined.contains("very long"),
            "should contain 'very long' in: {combined}"
        );
        assert!(
            combined.contains("wrap"),
            "should contain 'wrap' in: {combined}"
        );
        // And more lines than the non-wrapped baseline (header + blank + phase + task + blank = 5).
        assert!(lines.len() > 5, "expected wrapping to produce extra lines, got {}", lines.len());
    }

    #[test]
    fn build_render_lines_long_phase_wraps() {
        // Given a phase with a long description and narrow sidebar.
        let mut app = AppState::default();
        app.frontend.sidebar_width = 20;
        let session = app.session.active_session_mut();
        session
            .task_list_mut()
            .add_phase("Research and investigate the whole codebase");
        let list = session.task_list().clone();

        // When rendering.
        let lines = build_render_lines(&list, &app);

        // Then the full phase description is present.
        let combined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect();
        assert!(
            combined.contains("codebase"),
            "should contain 'codebase' in: {combined}"
        );
    }

    #[test]
    fn content_height_wraps_long_description() {
        // Given a task with a 60-char description and narrow sidebar.
        let mut app = AppState::default();
        app.frontend.sidebar_width = 20;
        let session = app.session.active_session_mut();
        let pid = session.task_list_mut().add_phase("Build");
        session
            .task_list_mut()
            .add_task(
                &pid,
                "This is a very long task description that should definitely wrap across multiple lines",
                TaskPosition::End,
            )
            .unwrap();
        let list = session.task_list().clone();

        // When computing height.
        let height = compute_height(&list, app.frontend.sidebar_width);

        // Then height accounts for wrapped lines (more than the flat 5 baseline).
        assert!(
            height > 5,
            "expected height > 5 due to wrapping, got {height}"
        );
    }

    #[test]
    fn content_height_minimum_sidebar_no_panic() {
        // Given minimum sidebar width (15 columns).
        let mut app = AppState::default();
        app.frontend.sidebar_width = 15;
        let session = app.session.active_session_mut();
        let pid = session.task_list_mut().add_phase("Build");
        session
            .task_list_mut()
            .add_task(&pid, "Write some code", TaskPosition::End)
            .unwrap();
        let list = session.task_list().clone();

        // When computing height - should not panic.
        let height = compute_height(&list, 15);

        // Then height is positive.
        assert!(height > 0, "expected non-zero height, got {height}");
    }

    #[test]
    fn empty_description_does_not_panic() {
        // Given a task with an empty description.
        let mut app = AppState::default();
        let session = app.session.active_session_mut();
        let pid = session.task_list_mut().add_phase("Build");
        session
            .task_list_mut()
            .add_task(&pid, "", TaskPosition::End)
            .unwrap();
        let list = session.task_list().clone();

        // When rendering and computing height - should not panic.
        let lines = build_render_lines(&list, &app);
        let height = compute_height(&list, app.frontend.sidebar_width);

        // Then results are valid.
        assert!(!lines.is_empty());
        assert!(height > 0);
    }

    #[test]
    fn exact_fit_does_not_wrap() {
        // Given a task description that exactly fills the available width.
        // sidebar_width = 30, TASK_INDENT = 6, so available = 24.
        // "123456789012345678901234" = 24 chars.
        let mut app = AppState::default();
        app.frontend.sidebar_width = 30;
        let session = app.session.active_session_mut();
        let pid = session.task_list_mut().add_phase("Build");
        session
            .task_list_mut()
            .add_task(&pid, "123456789012345678901234", TaskPosition::End)
            .unwrap();
        let list = session.task_list().clone();

        // When computing height.
        let height = compute_height(&list, 30);

        // Then the task contributes exactly 1 row (no wrapping).
        // Baseline: header(1) + blank(1) + phase(1) + task(1) + trailing_gap(1) = 5.
        assert_eq!(height, 5, "expected no wrapping for exact-fit description");
    }
}
