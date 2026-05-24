//! Workflow tab rendering — displays the workflow visualization.

use nullslop_domain::AppState;
use nullslop_domain::common::app_state::WorkflowUiState;
use nullslop_workflow_tui::widget::WorkflowWidget;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Block,
};

/// Renders the workflow tab content.
///
/// If a workflow is active, renders the `WorkflowWidget`.
/// Otherwise, renders a placeholder message.
pub fn render_workflow_tab(frame: &mut Frame<'_>, area: Rect, state: &AppState, tick: u8) {
    let Some(workflow) = state.workflow.active() else {
        render_no_workflow_placeholder(frame, area);
        return;
    };

    let snapshot = workflow.execution.snapshot();
    let viewport = viewport_from_ui(&state.frontend.workflow_ui);
    let widget = WorkflowWidget::new(&snapshot, &viewport, tick);
    frame.render_widget(widget, area);
}

/// Constructs a `ViewportState` from the persisted `WorkflowUiState`.
fn viewport_from_ui(ui: &WorkflowUiState) -> nullslop_workflow_tui::viewport::ViewportState {
    nullslop_workflow_tui::viewport::ViewportState {
        offset_x: ui.viewport_offset_x,
        offset_y: ui.viewport_offset_y,
        selected: ui.selected_node.clone(),
    }
}

/// Renders a placeholder when no workflow graph is available.
fn render_no_workflow_placeholder(frame: &mut Frame<'_>, area: Rect) {
    let line = Line::from(vec![
        Span::styled(
            " No workflow active — use ",
            Style::default().add_modifier(Modifier::DIM),
        ),
        Span::styled(
            "/workflow <name>",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " to start one",
            Style::default().add_modifier(Modifier::DIM),
        ),
    ]);
    frame.render_widget(
        ratatui::widgets::Paragraph::new(line).block(Block::default()),
        area,
    );
}
