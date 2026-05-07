//! Renders the workflow panel — step list with status indicators and detail view.
//!
//! When a workflow is active, displays a step list with per-step status indicators
//! and a yellow selection marker. Pressing `D` toggles a detail overlay showing
//! the selected step's title, status, model hint, flags, instructions, and outputs.
//! When no workflow is active, shows a dimmed "No active workflow." message.

use nullslop_component_ui::UiElement;
use nullslop_workflow::{ModelHint, StepDef, StepOutputDef, StepStatus, WorkflowState};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::AppState;

/// Solid yellow full block used as the selection indicator (same as dashboard).
const SELECTED_INDICATOR: &str = "\u{2588}\u{2588}";
/// Two spaces used as the unselected border.
const UNSELECTED_BORDER: &str = "  ";

/// Display element for the workflow panel.
#[derive(Debug)]
pub struct WorkflowPanelElement;

impl UiElement<AppState> for WorkflowPanelElement {
    fn name(&self) -> String {
        "workflow-panel".to_owned()
    }

    fn is_selectable(&self) -> bool {
        true
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        let Some(workflow) = state.active_session().workflow() else {
            render_no_workflow(frame, area);
            return;
        };

        let selected_index = state.workflow_panel.selected_index();
        let show_detail = state.workflow_panel.show_detail();

        if show_detail {
            render_with_detail(frame, area, workflow, selected_index);
        } else {
            render_list_only(frame, area, workflow, selected_index);
        }
    }
}

/// Renders the "No active workflow." message.
fn render_no_workflow(frame: &mut Frame<'_>, area: Rect) {
    let msg = Paragraph::new("No active workflow.")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::NONE));
    frame.render_widget(msg, area);
}

/// Renders the step list only (no detail pane).
fn render_list_only(
    frame: &mut Frame<'_>,
    area: Rect,
    workflow: &WorkflowState,
    selected_index: usize,
) {
    let step_ids = workflow.step_order();
    let lines = build_step_list(workflow, &step_ids, selected_index, area.width);
    render_with_scroll(frame, area, lines, workflow, selected_index);
}

/// Renders the step list (top portion) and detail pane (bottom portion).
fn render_with_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    workflow: &WorkflowState,
    selected_index: usize,
) {
    let step_ids = workflow.step_order();
    let available = area.height;

    // Reserve at least 8 rows for detail, at most 50% for list.
    // Reserve at least 8 rows for detail, at most 60% for list.
    #[expect(
        clippy::integer_division,
        reason = "u16 division is fine for layout sizing"
    )]
    let detail_rows = (20u16).min((available * 3 / 5).max(8));
    let list_rows = available.saturating_sub(detail_rows).max(4);
    let separator_row = 1;

    let [list_area, sep_area, detail_area] = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(list_rows),
        ratatui::layout::Constraint::Length(separator_row),
        ratatui::layout::Constraint::Min(detail_rows),
    ])
    .areas(area);

    // Render step list.
    let list_lines = build_step_list(workflow, &step_ids, selected_index, list_area.width);
    render_list_in_area(frame, list_area, list_lines);

    // Render separator.
    let separator = Paragraph::new("─".repeat(sep_area.width as usize))
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(separator, sep_area);

    // Render detail for selected step.
    let step_id = step_ids.get(selected_index);
    let detail_lines = step_id.and_then(|id| workflow.steps.get(id)).map_or_else(
        || {
            vec![Line::from(Span::styled(
                "No step selected.",
                Style::default().fg(Color::DarkGray),
            ))]
        },
        |s| build_detail_lines(&s.def, &s.status, &s.resolved_outputs),
    );

    let detail_widget = Paragraph::new(detail_lines).wrap(Wrap { trim: true });
    frame.render_widget(detail_widget, detail_area);
}

/// Builds the step list lines with status indicators and selection markers.
fn build_step_list(
    workflow: &WorkflowState,
    step_ids: &[String],
    selected_index: usize,
    _area_width: u16,
) -> Vec<Line<'static>> {
    let completed_count = step_ids
        .iter()
        .filter_map(|id| {
            workflow
                .steps
                .get(id)
                .filter(|s| s.status == StepStatus::Completed)
        })
        .count();
    let total = step_ids.len();

    let mut lines = Vec::new();

    // Header: workflow name + step progress.
    lines.push(Line::from(vec![
        Span::styled(
            format!("{} ", workflow.definition.name),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("— {completed_count}/{total} steps"),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    lines.push(Line::from(""));

    for (i, step_id) in step_ids.iter().enumerate() {
        let is_selected = i == selected_index;
        let step_state = workflow.steps.get(step_id);
        let (status_symbol, status_color) =
            step_state.map_or(("○", Color::DarkGray), |s| status_indicator(&s.status));

        let title = step_state.map_or(step_id.as_str(), |s| s.def.title.as_str());

        let border = if is_selected {
            Span::styled(SELECTED_INDICATOR, Style::default().fg(Color::Yellow))
        } else {
            Span::raw(UNSELECTED_BORDER)
        };

        let style = if is_selected {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        lines.push(Line::from(vec![
            border,
            Span::styled(
                format!(" {status_symbol} "),
                Style::default().fg(status_color),
            ),
            Span::styled(title.to_owned(), style),
        ]));

        // Blank line between steps (not after the last one).
        if i < step_ids.len() - 1 {
            lines.push(Line::from(""));
        }
    }

    lines
}

/// Renders step list with scroll clamping.
fn render_with_scroll(
    frame: &mut Frame<'_>,
    area: Rect,
    lines: Vec<Line<'static>>,
    _workflow: &WorkflowState,
    _selected_index: usize,
) {
    let total_lines = lines.len() as u16;
    let max_offset = total_lines.saturating_sub(area.height);
    // For now, use 0 scroll offset — scroll tracking will be enhanced later.
    let scroll_offset = max_offset;

    let widget = Paragraph::new(lines)
        .block(Block::default().borders(Borders::NONE))
        .scroll((scroll_offset, 0));
    frame.render_widget(widget, area);
}

/// Renders a list of lines in a given area with no scroll handling.
fn render_list_in_area(frame: &mut Frame<'_>, area: Rect, lines: Vec<Line<'static>>) {
    let total_lines = lines.len() as u16;
    let max_offset = total_lines.saturating_sub(area.height);
    let scroll_offset = max_offset;

    let widget = Paragraph::new(lines)
        .block(Block::default().borders(Borders::NONE))
        .scroll((scroll_offset, 0));
    frame.render_widget(widget, area);
}

/// Returns the status indicator symbol and color for a step status.
fn status_indicator(status: &StepStatus) -> (&'static str, Color) {
    match status {
        StepStatus::Completed => ("✓", Color::Green),
        StepStatus::Active => ("▶", Color::Cyan),
        StepStatus::Pending => ("○", Color::DarkGray),
        StepStatus::Stale => ("⚠", Color::Yellow),
        StepStatus::AwaitingInput => ("⏸", Color::Magenta),
    }
}

/// Returns a human-readable label for a step status.
fn status_label(status: &StepStatus) -> &'static str {
    match status {
        StepStatus::Completed => "Completed",
        StepStatus::Active => "Active",
        StepStatus::Pending => "Pending",
        StepStatus::Stale => "Stale",
        StepStatus::AwaitingInput => "Awaiting Input",
    }
}

/// Returns a short label for a model hint.
fn model_hint_label(hint: &ModelHint) -> &'static str {
    match hint {
        ModelHint::Small => "small",
        ModelHint::Medium => "medium",
        ModelHint::Large => "large",
        ModelHint::Exact { .. } => "exact",
    }
}

/// Builds the detail lines for a selected step.
fn build_detail_lines(
    def: &StepDef,
    status: &StepStatus,
    resolved_outputs: &std::collections::HashMap<String, String>,
) -> Vec<Line<'static>> {
    let (status_symbol, status_color) = status_indicator(status);
    let status_text = status_label(status);

    let mut lines = Vec::new();

    // Title (bold).
    lines.push(Line::from(Span::styled(
        def.title.clone(),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));

    // Status badge.
    lines.push(Line::from(vec![
        Span::styled(
            format!("{status_symbol} "),
            Style::default().fg(status_color),
        ),
        Span::styled(status_text.to_owned(), Style::default().fg(status_color)),
    ]));

    // Model hint.
    lines.push(Line::from(vec![
        Span::styled("model: ", Style::default().fg(Color::DarkGray)),
        Span::raw(model_hint_label(&def.model_hint)),
    ]));

    // Flags.
    let mut flags = Vec::new();
    if def.checkpoint {
        flags.push("checkpoint");
    }
    if def.requires_user_input {
        flags.push("requires-user-input");
    }
    if !flags.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("flags: ", Style::default().fg(Color::DarkGray)),
            Span::raw(flags.join(", ")),
        ]));
    }

    // Blank separator.
    lines.push(Line::from(""));

    // Instructions (truncated preview).
    lines.push(Line::from(Span::styled(
        "Instructions:",
        Style::default().fg(Color::DarkGray),
    )));
    // Show first 3 lines of instructions.
    let instruction_lines: Vec<&str> = def.instructions.lines().take(3).collect();
    for line in instruction_lines {
        lines.push(Line::from(format!("  {line}")));
    }
    if def.instructions.lines().count() > 3 {
        lines.push(Line::from(Span::styled(
            "  ...",
            Style::default().fg(Color::DarkGray),
        )));
    }

    // Outputs.
    if !def.outputs.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Outputs:",
            Style::default().fg(Color::DarkGray),
        )));

        for output in &def.outputs {
            let resolved = resolved_outputs.get(output.label());
            match output {
                StepOutputDef::File { label, path } => {
                    if let Some(value) = resolved {
                        lines.push(Line::from(format!("  {label}: {value}")));
                    } else {
                        lines.push(Line::from(vec![
                            Span::styled(format!("  {label}: "), Style::default()),
                            Span::styled(path.clone(), Style::default().fg(Color::DarkGray)),
                        ]));
                    }
                }
                StepOutputDef::Summary { label, value } => {
                    if let Some(resolved_value) = resolved {
                        lines.push(Line::from(format!("  {label}: {resolved_value}")));
                    } else {
                        lines.push(Line::from(vec![
                            Span::styled(format!("  {label}: "), Style::default()),
                            Span::styled(value.clone(), Style::default().fg(Color::DarkGray)),
                        ]));
                    }
                }
                StepOutputDef::Artifact { label, description } => {
                    lines.push(Line::from(format!("  {label}: {description}")));
                }
            }
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nullslop_workflow::{
        GuardExpr, ModelHint, StepDef, StepOutputDef, StepStatus, WorkflowDef, WorkflowState,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

    use super::*;
    use crate::AppState;

    /// Creates a workflow definition for testing.
    fn make_workflow(step_count: usize) -> WorkflowDef {
        let steps: Vec<StepDef> = (0..step_count)
            .map(|i| StepDef {
                id: format!("step-{i}"),
                title: format!("Step {i}"),
                instructions: format!("Instructions for step {i}"),
                model_hint: ModelHint::Small,
                checkpoint: false,
                requires_user_input: false,
                tools: vec![],
                guards: GuardExpr::None,
                outputs: vec![],
                depends_on: vec![],
            })
            .collect();

        WorkflowDef {
            version: 1,
            name: "test-workflow".to_owned(),
            description: "A test workflow".to_owned(),
            model_overrides: HashMap::new(),
            globals: HashMap::new(),
            steps,
        }
    }

    /// Creates a workflow definition with specific step configs.
    fn make_workflow_with_details() -> WorkflowDef {
        WorkflowDef {
            version: 1,
            name: "my-workflow".to_owned(),
            description: "A detailed workflow".to_owned(),
            model_overrides: HashMap::new(),
            globals: HashMap::new(),
            steps: vec![
                StepDef {
                    id: "step-0".to_owned(),
                    title: "Create Directory".to_owned(),
                    instructions:
                        "Ask the user for the directory name.\nCreate it.\nVerify.\nExtra line."
                            .to_owned(),
                    model_hint: ModelHint::Small,
                    checkpoint: true,
                    requires_user_input: true,
                    tools: vec![],
                    guards: GuardExpr::None,
                    outputs: vec![
                        StepOutputDef::File {
                            label: "Notes file".to_owned(),
                            path: "{{dir}}/notes.md".to_owned(),
                        },
                        StepOutputDef::Summary {
                            label: "Directory".to_owned(),
                            value: "{{dir_name}}".to_owned(),
                        },
                    ],
                    depends_on: vec![],
                },
                StepDef {
                    id: "step-1".to_owned(),
                    title: "Generate Background".to_owned(),
                    instructions: "Generate a background image.".to_owned(),
                    model_hint: ModelHint::Large,
                    checkpoint: false,
                    requires_user_input: false,
                    tools: vec![],
                    guards: GuardExpr::None,
                    outputs: vec![],
                    depends_on: vec![],
                },
            ],
        }
    }

    fn render_rows(
        element: &mut WorkflowPanelElement,
        state: &AppState,
        width: u16,
        height: u16,
    ) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, width, height);
        terminal
            .draw(|frame| {
                element.render(frame, area, state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| {
                        buffer
                            .cell((x, y))
                            .map_or(" ", ratatui::buffer::Cell::symbol)
                    })
                    .collect()
            })
            .collect()
    }

    fn load_state(def: WorkflowDef) -> AppState {
        let mut ws = WorkflowState::new(def);
        ws.start().unwrap();
        let mut state = AppState::default();
        state.active_session_mut().set_workflow(ws);
        state
    }

    #[test]
    fn name_returns_workflow_panel() {
        let element = WorkflowPanelElement;
        assert_eq!(element.name(), "workflow-panel");
    }

    #[test]
    fn render_no_workflow_shows_message() {
        let mut element = WorkflowPanelElement;
        let state = AppState::default();
        let rows = render_rows(&mut element, &state, 40, 10);
        assert!(rows[0].contains("No active workflow."));
    }

    #[test]
    fn render_step_list_shows_all_steps() {
        let mut element = WorkflowPanelElement;
        let state = load_state(make_workflow(3));
        let rows = render_rows(&mut element, &state, 60, 20);
        let combined = rows.join("\n");
        assert!(
            combined.contains("Step 0"),
            "should contain 'Step 0', got: {combined}"
        );
        assert!(
            combined.contains("Step 1"),
            "should contain 'Step 1', got: {combined}"
        );
        assert!(
            combined.contains("Step 2"),
            "should contain 'Step 2', got: {combined}"
        );
    }

    #[test]
    fn render_status_indicators() {
        // 4-step workflow: set each to a different status.
        let mut element = WorkflowPanelElement;
        let mut state = load_state(make_workflow(4));

        // step-0 Completed, step-1 Active, step-2 Pending (default), step-3 Stale.
        if let Some(step) = state
            .active_session_mut()
            .workflow_mut()
            .and_then(|w| w.steps.get_mut("step-0"))
        {
            step.status = StepStatus::Completed;
        }
        if let Some(step) = state
            .active_session_mut()
            .workflow_mut()
            .and_then(|w| w.steps.get_mut("step-1"))
        {
            step.status = StepStatus::Active;
        }
        // step-2 remains Pending by default.
        if let Some(step) = state
            .active_session_mut()
            .workflow_mut()
            .and_then(|w| w.steps.get_mut("step-3"))
        {
            step.status = StepStatus::Stale;
        }

        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 60, 24);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let combined: String = (0..24)
            .map(|y| {
                (0..60)
                    .map(|x| {
                        buffer
                            .cell((x, y))
                            .map_or(" ", ratatui::buffer::Cell::symbol)
                    })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Verify status indicators appear
        assert!(combined.contains("✓"), "should contain ✓ for Completed");
        assert!(combined.contains("▶"), "should contain ▶ for Active");
        assert!(combined.contains("○"), "should contain ○ for Pending");
        assert!(combined.contains("⚠"), "should contain ⚠ for Stale");
    }

    #[test]
    fn render_selected_step_has_yellow_marker() {
        let mut element = WorkflowPanelElement;
        let state = load_state(make_workflow(3));

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 60, 20);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // First step (index 0) should have yellow marker at columns 0-1.
        let buffer = terminal.backend().buffer();
        // Find the row that has the step title.
        // Header is row 0, blank row 1, step-0 at row 2.
        let cell0 = buffer.cell((0, 2)).expect("cell 0,2");
        assert_eq!(cell0.symbol(), "\u{2588}");
        assert_eq!(cell0.fg, Color::Yellow);
    }

    #[test]
    fn render_progress_header() {
        let mut element = WorkflowPanelElement;
        let state = load_state(make_workflow(3));
        let rows = render_rows(&mut element, &state, 60, 20);
        assert!(
            rows[0].contains("test-workflow"),
            "header should contain workflow name"
        );
        assert!(
            rows[0].contains("0/3 steps"),
            "header should show step progress"
        );
    }

    #[test]
    fn render_detail_shows_step_information() {
        let mut element = WorkflowPanelElement;
        let mut state = load_state(make_workflow_with_details());
        state.workflow_panel.toggle_detail();

        let rows = render_rows(&mut element, &state, 80, 30);
        let combined = rows.join("\n");

        // Detail should show title, status, model hint.
        assert!(
            combined.contains("Create Directory"),
            "detail should show step title"
        );
        assert!(combined.contains("model:"), "detail should show model hint");
        assert!(
            combined.contains("small"),
            "detail should show model hint value"
        );
    }

    #[test]
    fn render_detail_shows_outputs() {
        let mut element = WorkflowPanelElement;
        let mut state = load_state(make_workflow_with_details());
        state.workflow_panel.toggle_detail();

        // Give step-0 a resolved output.
        if let Some(step) = state
            .active_session_mut()
            .workflow_mut()
            .and_then(|w| w.steps.get_mut("step-0"))
        {
            step.resolved_outputs
                .insert("Directory".to_owned(), "my-dir".to_owned());
        }

        let rows = render_rows(&mut element, &state, 80, 50);
        let combined = rows.join("\n");

        assert!(
            combined.contains("Outputs:"),
            "detail should show outputs section, got: {combined}"
        );
        assert!(
            combined.contains("my-dir"),
            "detail should show resolved output value, got: {combined}"
        );
    }

    #[test]
    fn render_detail_shows_checkpoint_flag() {
        let mut element = WorkflowPanelElement;
        let mut state = load_state(make_workflow_with_details());
        state.workflow_panel.toggle_detail();

        let rows = render_rows(&mut element, &state, 80, 30);
        let combined = rows.join("\n");

        assert!(
            combined.contains("flags:"),
            "detail should show flags section"
        );
        assert!(
            combined.contains("checkpoint"),
            "detail should show checkpoint flag"
        );
        assert!(
            combined.contains("requires-user-input"),
            "detail should show requires-user-input flag"
        );
    }

    #[test]
    fn workflow_panel_element_is_selectable() {
        let element = WorkflowPanelElement;
        let selectable: &dyn UiElement<AppState> = &element;
        assert!(selectable.is_selectable());
    }
}
