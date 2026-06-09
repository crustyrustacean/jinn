//! [`TaskListSection`] - the task list sidebar section.
//!
//! Interactive section that displays the phased task list for the active session.
//! Phases are collapsed by default; the focused phase expands to show its tasks.
//! The section is hidden when the task list is empty.

use std::borrow::Cow;

use crate::common::app_state::AppState;
use crate::common::render_ctx::RenderCtx;
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
/// Phases are collapsed when unfocused; the selected phase expands when focused.
#[derive(Debug)]
pub struct TaskListSection;

/// State for the task list sidebar section.
///
/// Tracks which phase is selected (has cursor). `None` means the section
/// is unfocused — all phases are collapsed.
#[derive(Debug, Clone, Default)]
pub struct TaskListSectionState {
    /// Index into the task list's phases vector.
    /// `None` when the section is unfocused.
    pub selected_phase_index: Option<usize>,
}

/// Navigate within the task list section.
///
/// Moves the phase cursor up/down. Returns `Exhausted` at boundaries.
pub fn navigate(intent: &SidebarIntent, state: &mut AppState) -> SectionNavResult {
    let Some(index) = state.frontend.task_list_section.selected_phase_index else {
        return SectionNavResult::Exhausted;
    };
    let phase_count = state.active_session().task_list().phases().len();
    if phase_count == 0 {
        return SectionNavResult::Exhausted;
    }
    match intent {
        SidebarIntent::MoveDown => {
            if index + 1 < phase_count {
                state.frontend.task_list_section.selected_phase_index = Some(index + 1);
                SectionNavResult::Moved
            } else {
                SectionNavResult::Exhausted
            }
        }
        SidebarIntent::MoveUp => {
            if index > 0 {
                state.frontend.task_list_section.selected_phase_index = Some(index - 1);
                SectionNavResult::Moved
            } else {
                SectionNavResult::Exhausted
            }
        }
        SidebarIntent::Action(_) => SectionNavResult::Exhausted,
    }
}

/// Place the cursor on this section.
///
/// Sets the selected phase index based on entry direction.
pub fn receive_cursor(state: &mut AppState, enter_from: EnterFrom) {
    let phase_count = state.active_session().task_list().phases().len();
    if phase_count == 0 {
        return;
    }
    state.frontend.task_list_section.selected_phase_index = Some(match enter_from {
        EnterFrom::Top => 0,
        EnterFrom::Bottom => phase_count - 1,
    });
}

impl SidebarSection for TaskListSection {
    fn id(&self) -> SidebarSectionId {
        SidebarSectionId::TaskList
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
        let state = ctx.state;
        let list = state.active_session().task_list();
        if list.is_empty() {
            return;
        }

        let lines = build_render_lines(list, state);
        let widget = Paragraph::new(lines);
        frame.render_widget(widget, area);
    }

    fn content_height(&self, ctx: &RenderCtx) -> u16 {
        let state = ctx.state;
        let list = state.active_session().task_list();
        if list.is_empty() {
            return 0;
        }
        compute_height(list, state)
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

/// Returns the expanded phase index if the sidebar is focused on the task list section.
fn expanded_phase_index(state: &AppState) -> Option<usize> {
    if state.frontend.scope_stack.sidebar_section() == Some(SidebarSectionId::TaskList) {
        state.frontend.task_list_section.selected_phase_index
    } else {
        None
    }
}

/// Builds the render lines for a task list.
///
/// When unfocused (no expanded phase), renders only phase headers with `▸` indicators.
/// When focused, expands the selected phase showing its tasks with a `▾` indicator.
fn build_render_lines(list: &TaskList, state: &AppState) -> Vec<Line<'static>> {
    let theme = &state.frontend.theme;
    let sidebar_width = state.frontend.sidebar_width as usize;
    let expanded = expanded_phase_index(state);
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

    for (phase_idx, phase) in list.phases().iter().enumerate() {
        let is_expanded = expanded == Some(phase_idx);
        let is_selected = expanded == Some(phase_idx);

        // Phase header with collapse indicator.
        let indicator = if is_expanded {
            "\u{25BE} " // ▾ expanded
        } else {
            "\u{25B8} " // ▸ collapsed
        };
        let phase_width = sidebar_width.saturating_sub(PHASE_INDENT + indicator.len());
        let mut phase_style = Style::default()
            .fg(theme.muted_text)
            .add_modifier(Modifier::BOLD);
        // Highlight the selected phase header with reversed colors.
        if is_selected {
            phase_style = phase_style.add_modifier(Modifier::REVERSED);
        }
        let wrapped = wrap_description(phase.description(), phase_width);
        for (i, segment) in wrapped.iter().enumerate() {
            let prefix = if i == 0 {
                format!("  {indicator}{segment}")
            } else {
                format!("    {}{segment}", " ".repeat(indicator.len()))
            };
            lines.push(Line::from(Span::styled(prefix, phase_style)));
        }

        // Only render tasks for the expanded phase.
        if is_expanded {
            if phase.is_empty() {
                lines.push(Line::from(Span::styled(
                    "    (no tasks)",
                    Style::default().fg(theme.muted_text),
                )));
            } else {
                let task_width = sidebar_width.saturating_sub(TASK_INDENT);
                for task in phase.tasks() {
                    let (indicator, style) = match task.status() {
                        TaskStatus::Pending => {
                            ("\u{25CB} ", Style::default().fg(theme.primary_text)) // ○
                        }
                        TaskStatus::Completed => {
                            ("\u{2713} ", Style::default().fg(theme.muted_text)) // ✓
                        }
                        TaskStatus::Postponed => {
                            ("\u{25BC} ", Style::default().fg(theme.muted_text)) // ▼
                        }
                        TaskStatus::Cancelled => {
                            // Cancelled tasks are shown with strikethrough.
                            (
                                "\u{2717} ", // ✗
                                Style::default()
                                    .fg(theme.muted_text)
                                    .add_modifier(Modifier::CROSSED_OUT),
                            )
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
                            lines.push(Line::from(Span::styled(format!("      {segment}"), style)));
                        }
                    }
                }
            }
        }
    }

    lines
}
/// Computes the content height for a non-empty task list.
fn compute_height(list: &TaskList, state: &AppState) -> u16 {
    let sidebar_width = state.frontend.sidebar_width as usize;
    let expanded = expanded_phase_index(state);
    let mut height: usize = 0;

    // Header + blank.
    height += 2;

    for (phase_idx, phase) in list.phases().iter().enumerate() {
        let is_expanded = expanded == Some(phase_idx);

        // Phase header - count wrapped lines.
        // Account for indicator ("▾ " or "▸ " = 2 chars) + indent.
        let indicator_len = 2; // "▾ " or "▸ "
        let phase_width = sidebar_width.saturating_sub(PHASE_INDENT + indicator_len);
        height += wrap_description(phase.description(), phase_width).len();

        // Only count task lines for the expanded phase.
        if is_expanded {
            if phase.is_empty() {
                height += 1; // "(no tasks)"
            } else {
                let task_width = sidebar_width.saturating_sub(TASK_INDENT);
                for task in phase.tasks() {
                    height += wrap_description(task.description(), task_width).len();
                }
            }
        }
    }

    height as u16
}

#[cfg(test)]
mod tests {
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::string_slice, clippy::uninlined_format_args, reason = "test code")]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::focus::FocusScope;
    use crate::common::render_ctx::RenderCtx;
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

    /// Helper: set up focus on a specific phase so it expands.
    fn setup_focused_on_phase(app: &mut AppState, phase_index: usize) {
        app.frontend.scope_stack.push(FocusScope::SidebarTaskList);
        app.frontend.task_list_section.selected_phase_index = Some(phase_index);
    }

    /// Extract all text content from render lines.
    fn extract_text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect()
    }

    #[test]
    fn content_height_is_zero_when_empty() {
        let app = AppState::default();
        let section = TaskListSection;
        assert_eq!(section.content_height(&{ RenderCtx::new(&app) }), 0);
    }

    #[test]
    fn content_height_is_nonzero_when_has_phases() {
        let app = setup_with_tasks();
        let section = TaskListSection;
        let height = section.content_height(&{ RenderCtx::new(&app) });
        assert!(height > 0, "expected non-zero height, got {height}");
    }

    // --- Navigation tests ---

    #[test]
    fn navigate_returns_exhausted_without_selection() {
        let mut app = AppState::default();
        let result = navigate(&SidebarIntent::MoveDown, &mut app);
        assert_eq!(result, SectionNavResult::Exhausted);
    }

    #[test]
    fn navigate_moves_down_within_bounds() {
        let mut app = setup_with_tasks();
        setup_focused_on_phase(&mut app, 0);
        let result = navigate(&SidebarIntent::MoveDown, &mut app);
        assert_eq!(result, SectionNavResult::Moved);
        assert_eq!(app.frontend.task_list_section.selected_phase_index, Some(1));
    }

    #[test]
    fn navigate_moves_up_within_bounds() {
        let mut app = setup_with_tasks();
        setup_focused_on_phase(&mut app, 1);
        let result = navigate(&SidebarIntent::MoveUp, &mut app);
        assert_eq!(result, SectionNavResult::Moved);
        assert_eq!(app.frontend.task_list_section.selected_phase_index, Some(0));
    }

    #[test]
    fn navigate_exhausted_at_bottom() {
        let mut app = setup_with_tasks();
        setup_focused_on_phase(&mut app, 1); // last phase
        let result = navigate(&SidebarIntent::MoveDown, &mut app);
        assert_eq!(result, SectionNavResult::Exhausted);
    }

    #[test]
    fn navigate_exhausted_at_top() {
        let mut app = setup_with_tasks();
        setup_focused_on_phase(&mut app, 0); // first phase
        let result = navigate(&SidebarIntent::MoveUp, &mut app);
        assert_eq!(result, SectionNavResult::Exhausted);
    }

    #[test]
    fn receive_cursor_sets_first_phase_from_top() {
        let mut app = setup_with_tasks();
        receive_cursor(&mut app, EnterFrom::Top);
        assert_eq!(app.frontend.task_list_section.selected_phase_index, Some(0));
    }

    #[test]
    fn receive_cursor_sets_last_phase_from_bottom() {
        let mut app = setup_with_tasks();
        receive_cursor(&mut app, EnterFrom::Bottom);
        assert_eq!(app.frontend.task_list_section.selected_phase_index, Some(1));
    }

    #[test]
    fn receive_cursor_no_panic_on_empty_list() {
        let mut app = AppState::default();
        receive_cursor(&mut app, EnterFrom::Top);
        assert_eq!(app.frontend.task_list_section.selected_phase_index, None);
    }

    #[test]
    fn id_returns_task_list() {
        let section = TaskListSection;
        assert_eq!(section.id(), SidebarSectionId::TaskList);
    }

    // --- Collapsed rendering tests ---

    #[test]
    fn collapsed_rendering_shows_no_tasks() {
        // When unfocused, task descriptions should NOT appear.
        let app = setup_with_tasks();
        let list = app.session.active_session().task_list().clone();
        let lines = build_render_lines(&list, &app);
        let combined = extract_text(&lines);
        assert!(
            combined.contains("Research"),
            "should contain phase: Research"
        );
        assert!(combined.contains("Build"), "should contain phase: Build");
        assert!(
            !combined.contains("Read docs"),
            "collapsed should NOT contain task: Read docs"
        );
        assert!(
            !combined.contains("Write code"),
            "collapsed should NOT contain task: Write code"
        );
    }

    #[test]
    fn collapsed_shows_collapse_indicator() {
        let app = setup_with_tasks();
        let list = app.session.active_session().task_list().clone();
        let lines = build_render_lines(&list, &app);
        let combined = extract_text(&lines);
        assert!(
            combined.contains('\u{25B8}'),
            "collapsed phases should show \u{25B8} indicator"
        );
    }

    #[test]
    fn no_blank_lines_between_phases() {
        let app = setup_with_tasks();
        let list = app.session.active_session().task_list().clone();
        let lines = build_render_lines(&list, &app);
        // No line should be empty (blank) - every line should have content.
        for (i, line) in lines.iter().enumerate() {
            // The header separator blank (line index 1) is OK.
            // But between phases there should be no blank lines.
            if i == 1 {
                continue; // blank after header is expected
            }
            let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
            assert!(
                !text.trim().is_empty(),
                "line {i} should not be blank between phases: {text:?}"
            );
        }
    }

    #[test]
    fn selected_phase_header_has_reversed_modifier() {
        let mut app = setup_with_tasks();
        setup_focused_on_phase(&mut app, 0);
        let list = app.session.active_session().task_list().clone();
        let lines = build_render_lines(&list, &app);
        // Find a line containing the first phase name.
        let has_reversed = lines.iter().any(|line| {
            let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
            text.contains("Research")
                && line
                    .spans
                    .iter()
                    .any(|s| s.style.add_modifier.contains(Modifier::REVERSED))
        });
        assert!(
            has_reversed,
            "selected phase header should have REVERSED modifier for cursor highlight"
        );
    }

    // --- Expanded rendering tests ---
    // --- Expanded rendering tests ---

    #[test]
    fn expanded_rendering_shows_tasks_for_selected_phase() {
        let mut app = setup_with_tasks();
        setup_focused_on_phase(&mut app, 0); // expand Research phase
        let list = app.session.active_session().task_list().clone();
        let lines = build_render_lines(&list, &app);
        let combined = extract_text(&lines);
        // Research phase tasks should be visible.
        assert!(
            combined.contains("Read docs"),
            "expanded phase should show task: Read docs"
        );
        assert!(
            combined.contains("Call API"),
            "expanded phase should show task: Call API"
        );
        // Build phase tasks should NOT be visible (collapsed).
        assert!(
            !combined.contains("Write code"),
            "non-expanded phase should NOT show task: Write code"
        );
    }

    #[test]
    fn expanded_shows_expand_indicator() {
        let mut app = setup_with_tasks();
        setup_focused_on_phase(&mut app, 0);
        let list = app.session.active_session().task_list().clone();
        let lines = build_render_lines(&list, &app);
        let combined = extract_text(&lines);
        assert!(
            combined.contains('\u{25BE}'),
            "expanded phase should show \u{25BE} indicator"
        );
    }

    #[test]
    fn expanded_shows_pending_indicator() {
        let mut app = setup_with_tasks();
        setup_focused_on_phase(&mut app, 0);
        let list = app.session.active_session().task_list().clone();
        let lines = build_render_lines(&list, &app);
        let combined = extract_text(&lines);
        assert!(
            combined.contains('\u{25CB}'),
            "should contain pending indicator \u{25CB}"
        );
    }

    #[test]
    fn expanded_shows_completed_indicator() {
        let mut app = AppState::default();
        let session = app.session.active_session_mut();
        let pid = session.task_list_mut().add_phase("Build");
        let tid = session
            .task_list_mut()
            .add_task(&pid, "Write code", TaskPosition::End)
            .unwrap();
        session.task_list_mut().complete_task(&tid).unwrap();
        setup_focused_on_phase(&mut app, 0);
        let list = app.session.active_session().task_list().clone();
        let lines = build_render_lines(&list, &app);
        let combined = extract_text(&lines);
        assert!(
            combined.contains('\u{2713}'),
            "should contain completed indicator \u{2713}"
        );
    }

    #[test]
    fn expanded_shows_postponed_indicator() {
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
            .postpone_task(&t1, TaskPosition::After(t2))
            .unwrap();

        setup_focused_on_phase(&mut app, 0); // expand Research
        let list = app.session.active_session().task_list().clone();
        let lines = build_render_lines(&list, &app);
        let combined = extract_text(&lines);

        // Research phase should show postponed task with ▼.
        assert!(
            combined.contains('\u{25BC}'),
            "should contain postponed indicator \u{25BC}"
        );
    }

    // --- Cancelled task tests ---

    #[test]
    fn cancelled_task_visible_with_indicator() {
        let mut app = AppState::default();
        let session = app.session.active_session_mut();
        let pid = session.task_list_mut().add_phase("Build");
        let tid = session
            .task_list_mut()
            .add_task(&pid, "Bad idea", TaskPosition::End)
            .unwrap();
        session.task_list_mut().cancel_task(&tid).unwrap();
        setup_focused_on_phase(&mut app, 0);
        let list = app.session.active_session().task_list().clone();
        let lines = build_render_lines(&list, &app);
        let combined = extract_text(&lines);

        // Cancelled task should be visible with ✗ indicator.
        assert!(
            combined.contains('\u{2717}'),
            "should contain cancelled indicator \u{2717}"
        );
        assert!(
            combined.contains("Bad idea"),
            "cancelled task description should be visible"
        );
    }

    #[test]
    fn cancelled_task_has_crossed_out_modifier() {
        let mut app = AppState::default();
        let session = app.session.active_session_mut();
        let pid = session.task_list_mut().add_phase("Build");
        let tid = session
            .task_list_mut()
            .add_task(&pid, "Bad idea", TaskPosition::End)
            .unwrap();
        session.task_list_mut().cancel_task(&tid).unwrap();
        setup_focused_on_phase(&mut app, 0);
        let list = app.session.active_session().task_list().clone();
        let lines = build_render_lines(&list, &app);

        // Find a line containing the cancelled task and check it has CROSSED_OUT.
        let has_crossed_out = lines.iter().any(|line| {
            let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
            text.contains("Bad idea")
                && line
                    .spans
                    .iter()
                    .any(|s| s.style.add_modifier.contains(Modifier::CROSSED_OUT))
        });
        assert!(
            has_crossed_out,
            "cancelled task should have CROSSED_OUT modifier"
        );
    }

    // --- wrap_description tests ---

    #[test]
    fn wrap_description_short_text_no_wrap() {
        let result = wrap_description("hello", 20);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "hello");
    }

    #[test]
    fn wrap_description_long_text_wraps() {
        let text = "This is a very long description that should wrap";
        let result = wrap_description(text, 15);
        assert!(result.len() > 1, "expected wrapping, got {result:?}");
        let joined = result.join(" ");
        assert!(joined.contains("very long"));
    }

    #[test]
    fn wrap_description_zero_width_returns_original() {
        let result = wrap_description("hello", 0);
        assert_eq!(result, vec!["hello".to_owned()]);
    }

    #[test]
    fn wrap_description_empty_text_returns_single_empty() {
        let result = wrap_description("", 20);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "");
    }

    // --- Word-wrap integration tests ---

    #[test]
    fn build_render_lines_long_task_wraps() {
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
        setup_focused_on_phase(&mut app, 0);

        let lines = build_render_lines(&list, &app);
        let combined = extract_text(&lines);
        assert!(
            combined.contains("very long"),
            "should contain 'very long' in: {combined}"
        );
        assert!(
            combined.contains("wrap"),
            "should contain 'wrap' in: {combined}"
        );
        // More lines than collapsed baseline (header + blank + 2 phases + blank = 5).
        assert!(
            lines.len() > 5,
            "expected wrapping to produce extra lines, got {}",
            lines.len()
        );
    }

    #[test]
    fn build_render_lines_long_phase_wraps() {
        let mut app = AppState::default();
        app.frontend.sidebar_width = 20;
        let session = app.session.active_session_mut();
        session
            .task_list_mut()
            .add_phase("Research and investigate the whole codebase");
        let list = session.task_list().clone();

        let lines = build_render_lines(&list, &app);
        let combined = extract_text(&lines);
        assert!(
            combined.contains("codebase"),
            "should contain 'codebase' in: {combined}"
        );
    }

    #[test]
    fn content_height_wraps_long_description() {
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
        setup_focused_on_phase(&mut app, 0);

        let height = compute_height(&list, &app);
        // Collapsed baseline: header(1) + blank(1) + phase(1) + trailing_gap(1) = 4.
        // With expansion, height should be > 4 due to task wrapping.
        assert!(
            height > 4,
            "expected height > 4 due to wrapping, got {height}"
        );
    }

    #[test]
    fn content_height_minimum_sidebar_no_panic() {
        let mut app = AppState::default();
        app.frontend.sidebar_width = 15;
        let session = app.session.active_session_mut();
        let pid = session.task_list_mut().add_phase("Build");
        session
            .task_list_mut()
            .add_task(&pid, "Write some code", TaskPosition::End)
            .unwrap();
        let list = session.task_list().clone();

        // Should not panic.
        let height = compute_height(&list, &app);
        assert!(height > 0, "expected non-zero height, got {height}");
    }

    #[test]
    fn empty_description_does_not_panic() {
        let mut app = AppState::default();
        let session = app.session.active_session_mut();
        let pid = session.task_list_mut().add_phase("Build");
        session
            .task_list_mut()
            .add_task(&pid, "", TaskPosition::End)
            .unwrap();
        let list = session.task_list().clone();
        setup_focused_on_phase(&mut app, 0);

        let lines = build_render_lines(&list, &app);
        let height = compute_height(&list, &app);
        assert!(!lines.is_empty());
        assert!(height > 0);
    }

    #[test]
    fn exact_fit_does_not_wrap_collapsed() {
        // When collapsed (no focus), a phase description that exactly fits should
        // contribute exactly 1 row for the phase header.
        // sidebar_width = 30, PHASE_INDENT(2) + indicator_len(2) = 4, so available = 26.
        // "12345678901234567890123456" = 26 chars.
        let mut app = AppState::default();
        app.frontend.sidebar_width = 30;
        let session = app.session.active_session_mut();
        session
            .task_list_mut()
            .add_phase("12345678901234567890123456");
        let list = session.task_list().clone();

        let height = compute_height(&list, &app);
        // Collapsed: header(1) + blank(1) + phase(1) = 3.
        assert_eq!(height, 3, "expected no wrapping for exact-fit description");
    }

    #[test]
    fn content_height_collapsed_vs_expanded() {
        let app_collapsed = setup_with_tasks();
        let list = app_collapsed.session.active_session().task_list().clone();
        let collapsed_height = compute_height(&list, &app_collapsed);

        let mut app_expanded = setup_with_tasks();
        setup_focused_on_phase(&mut app_expanded, 0);
        let list2 = app_expanded.session.active_session().task_list().clone();
        let expanded_height = compute_height(&list2, &app_expanded);

        assert!(
            expanded_height > collapsed_height,
            "expanded height ({expanded_height}) should be > collapsed height ({collapsed_height})"
        );
    }
}
