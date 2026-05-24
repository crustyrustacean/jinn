//! Workflow tab rendering — displays the workflow visualization.
//!
//! Renders the graph widget, status line, inspector popup, and cancel prompt.

use nullslop_domain::AppState;
use nullslop_domain::common::app_state::WorkflowUiState;
use nullslop_workflow_tui::widget::WorkflowWidget;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

/// Renders the workflow tab content.
///
/// If a workflow is active, renders the `WorkflowWidget`, status line,
/// inspector popup (if open), and cancel prompt (if showing).
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

    // Status line at the bottom of the workflow area.
    render_status_line(frame, area, state);

    // Cancel prompt overlay.
    if state.frontend.workflow_ui.cancel_prompt {
        render_cancel_prompt(frame, area);
    }

    // Inspector popup overlay.
    if state.frontend.workflow_ui.inspector_open {
        render_inspector(frame, area, state);
    }
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
        Paragraph::new(line).block(Block::default()),
        area,
    );
}

/// Renders the status line at the bottom of the workflow tab.
///
/// Shows the selected node's name, status, and port counts.
/// If no node is selected, shows a navigation hint.
fn render_status_line(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let ui = &state.frontend.workflow_ui;
    let line = match &ui.selected_node {
        Some(name) => {
            let Some(workflow) = state.workflow.active() else {
                return;
            };
            let snapshot = workflow.execution.snapshot();
            let status_str = snapshot
                .status_of(name)
                .map(|s| format!("{s:?}"))
                .unwrap_or_else(|| "Unknown".to_owned());
            let ns = snapshot.node_state(name);
            let in_count = ns
                .and_then(|s| s.inputs.as_ref())
                .map(|p| p.len())
                .unwrap_or(0);
            let out_count = ns
                .and_then(|s| s.outputs.as_ref())
                .map(|p| p.len())
                .unwrap_or(0);
            Line::from(vec![
                Span::styled(" [", Style::default().add_modifier(Modifier::DIM)),
                Span::styled(
                    name.as_str(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled("] ", Style::default().add_modifier(Modifier::DIM)),
                Span::styled(
                    format!("{status_str}"),
                    Style::default().add_modifier(Modifier::DIM),
                ),
                Span::styled(
                    format!(" │ in:{in_count} │ out:{out_count}"),
                    Style::default().add_modifier(Modifier::DIM),
                ),
            ])
        }
        None => Line::from(Span::styled(
            " No node selected — use j/k to navigate",
            Style::default().add_modifier(Modifier::DIM),
        )),
    };

    let status_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    frame.render_widget(Paragraph::new(line), status_area);
}

/// Renders the cancel confirmation prompt as a centered overlay.
fn render_cancel_prompt(frame: &mut Frame<'_>, area: Rect) {
    let prompt_text = " Press ESC again to cancel workflow ";
    let text_width = u16::try_from(prompt_text.len()).unwrap_or(area.width);
    let popup_width = text_width.saturating_add(2).min(area.width);
    let x = area.x + area.width.saturating_sub(popup_width) / 2;
    let y = area.y + area.height.saturating_sub(1) / 2;

    let popup_area = Rect {
        x,
        y,
        width: popup_width,
        height: 1,
    };

    frame.render_widget(Clear, popup_area);
    let prompt = Paragraph::new(Span::styled(
        prompt_text,
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(ratatui::style::Color::Yellow),
    ));
    frame.render_widget(prompt, popup_area);
}

/// Renders the sticky inspector popup overlay.
///
/// Shows the selected node's details: name, status, config, inputs, outputs.
/// The popup is anchored to the top-right of the workflow area.
fn render_inspector(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let ui = &state.frontend.workflow_ui;
    let Some(node_name) = &ui.selected_node else {
        return;
    };

    let Some(workflow) = state.workflow.active() else {
        return;
    };
    let snapshot = workflow.execution.snapshot();
    let node_state = snapshot.node_state(node_name);
    let status = snapshot
        .status_of(node_name)
        .map(|s| format!("{s:?}"))
        .unwrap_or_else(|| "Unknown".to_owned());

    // Build content lines.
    let mut lines: Vec<Line<'_>> = vec![];

    // Status line.
    lines.push(Line::from(vec![
        Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(&status),
    ]));

    // Config section.
    if let Some(ns) = &node_state {
        if let Some(config) = &ns.config {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Config",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            let config_str = format!("{config}");
            for line in config_str.lines().take(5) {
                let truncated: String = line.chars().take(60).collect();
                lines.push(Line::from(Span::styled(
                    format!("  {truncated}"),
                    Style::default().add_modifier(Modifier::DIM),
                )));
            }
        }
    }

    // Inputs section.
    if let Some(ns) = &node_state {
        if let Some(inputs) = &ns.inputs {
            if !inputs.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Inputs",
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                for (name, value) in inputs.iter() {
                    let summary = port_value_summary(value);
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {name}: "), Style::default()),
                        Span::styled(summary, Style::default().add_modifier(Modifier::DIM)),
                    ]));
                }
            }
        }
    }

    // Outputs section.
    if let Some(ns) = &node_state {
        if let Some(outputs) = &ns.outputs {
            if !outputs.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Outputs",
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                for (name, value) in outputs.iter() {
                    let summary = port_value_summary(value);
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {name}: "), Style::default()),
                        Span::styled(summary, Style::default().add_modifier(Modifier::DIM)),
                    ]));
                }
            }
        }
    }

    // Footer with keybinds.
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "i close · ↑↓ scroll · r re-run",
        Style::default()
            .add_modifier(Modifier::DIM)
            .fg(ratatui::style::Color::DarkGray),
    )));

    // Compute popup dimensions.
    let content_height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let popup_width = (u16::try_from(area.width * 50 / 100).unwrap_or(30)).max(30).min(60);
    let desired_height = content_height.saturating_add(2); // +2 for borders
    let max_height = area.height / 2;
    let popup_height = desired_height.min(max_height).max(5);

    // Anchor to top-right.
    let popup_x = area.x + area.width.saturating_sub(popup_width);
    let popup_y = area.y;

    let popup_area = Rect {
        x: popup_x,
        y: popup_y,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(Span::styled(
            format!(" {node_name} "),
            Style::default().fg(ratatui::style::Color::Cyan),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ratatui::style::Color::DarkGray));
    frame.render_widget(block, popup_area);

    // Inner area (inside borders).
    let inner_area = Rect {
        x: popup_x + 1,
        y: popup_y + 1,
        width: popup_width.saturating_sub(2),
        height: popup_height.saturating_sub(2),
    };

    if inner_area.width == 0 || inner_area.height == 0 {
        return;
    }

    // Render content with scroll offset.
    let scroll_offset = ui.inspector_scroll;
    let content = Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((scroll_offset, 0));
    frame.render_widget(content, inner_area);
}

/// Produces a short summary of a `PortValue` for display.
fn port_value_summary(value: &nullslop_workflow::port::PortValue) -> String {
    match value {
        nullslop_workflow::port::PortValue::String(s) => {
            if s.len() > 80 {
                let truncated: String = s.chars().take(77).collect();
                format!("{truncated}...")
            } else {
                s.clone()
            }
        }
        nullslop_workflow::port::PortValue::Json(v) => {
            let s = serde_json::to_string(v).unwrap_or_else(|_| format!("{v}"));
            if s.len() > 200 {
                let truncated: String = s.chars().take(197).collect();
                format!("{truncated}...")
            } else {
                s
            }
        }
    }
}
