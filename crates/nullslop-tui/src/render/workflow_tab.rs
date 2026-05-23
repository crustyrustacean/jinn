//! Workflow tab rendering — displays the workflow visualization.

use nullslop_domain::AppState;
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

    let Some(ref graph) = workflow.graph_render_copy else {
        render_no_workflow_placeholder(frame, area);
        return;
    };

    // Render the workflow widget.
    let viewport = nullslop_workflow_tui::viewport::ViewportState::default();
    let widget = WorkflowWidget::new(graph, &workflow.statuses, &viewport, tick);
    frame.render_widget(widget, area);
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
