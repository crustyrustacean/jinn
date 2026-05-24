//! Workflow tab rendering — displays the workflow visualization.
//!
//! Renders the graph widget, status line, inspector popup, and cancel prompt.

use nullslop_domain::AppState;
use nullslop_domain::common::app_state::WorkflowUiState;
use nullslop_domain::feat::session::chat_entry::ChatEntryKind;
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
                .map_or_else(|| "Unknown".to_owned(), |s| format!("{s:?}"));
            let ns = snapshot.node_state(name);
            let in_count = ns
                .and_then(|s| s.inputs.as_ref())
                .map_or(0, |p| p.len());
            let out_count = ns
                .and_then(|s| s.outputs.as_ref())
                .map_or(0, |p| p.len());
            Line::from(vec![
                Span::styled(" [", Style::default().add_modifier(Modifier::DIM)),
                Span::styled(
                    name.as_str(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled("] ", Style::default().add_modifier(Modifier::DIM)),
                Span::raw(status_str),
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

/// Builds the content lines for the inspector popup.
fn build_inspector_lines(
    node_name: &str,
    node_state: Option<&nullslop_workflow::execution::NodeState>,
    status: &str,
    state: &AppState,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'_>> = vec![];

    // Status line.
    lines.push(Line::from(vec![
        Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(status.to_owned()),
    ]));

    // Config section.
    if let Some(ns) = node_state
        && let Some(config) = &ns.config
    {
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

    // Inputs section.
    if let Some(ns) = node_state
        && let Some(inputs) = &ns.inputs
        && !inputs.is_empty()
    {
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

    // Outputs section.
    if let Some(ns) = node_state
        && let Some(outputs) = &ns.outputs
        && !outputs.is_empty()
    {
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

    // Session section — look up node→session mapping and render history.
    if let Some(session) = lookup_node_session(state, node_name) {
        append_session_lines(&mut lines, session);
    }

    // Footer with keybinds.
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "i close · ↑↓ scroll · r re-run",
        Style::default()
            .add_modifier(Modifier::DIM)
            .fg(ratatui::style::Color::DarkGray),
    )));
    lines
}

/// Looks up the session associated with a workflow node.
fn lookup_node_session<'a>(
    state: &'a AppState,
    node_name: &str,
) -> Option<&'a nullslop_domain::feat::session::chat_session::ChatSessionState> {
    let workflow = state.workflow.active()?;
    let session_id = workflow.node_sessions.get(node_name)?;
    state.session.get(session_id)
}

/// Appends session history lines to the inspector output.
fn append_session_lines(lines: &mut Vec<Line<'static>>, session: &nullslop_domain::feat::session::chat_session::ChatSessionState) {
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Session",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for entry in session.history() {
        match &entry.kind {
            ChatEntryKind::User { display, .. } => {
                render_truncated_lines(lines, &format!("> {display}"), 80);
            }
            ChatEntryKind::Assistant(text) => {
                render_truncated_lines(lines, &format!("  {text}"), 80);
            }
            ChatEntryKind::System(text) => {
                render_truncated_lines(lines, &format!("  [system] {text}"), 80);
            }
            ChatEntryKind::Error(text) => {
                render_truncated_lines(lines, &format!("  [error] {text}"), 80);
            }
            ChatEntryKind::Thinking(text) => {
                render_truncated_lines(lines, &format!("  [thinking] {text}"), 80);
            }
            ChatEntryKind::ToolCall { name, .. } => {
                lines.push(Line::from(Span::styled(
                    format!("  [tool:{name}]"),
                    Style::default().add_modifier(Modifier::DIM),
                )));
            }
            ChatEntryKind::ToolResult { name, .. } => {
                lines.push(Line::from(Span::styled(
                    format!("  [result:{name}]"),
                    Style::default().add_modifier(Modifier::DIM),
                )));
            }
            ChatEntryKind::Skill { name, .. } => {
                lines.push(Line::from(Span::styled(
                    format!("  [skill:{name}]"),
                    Style::default().add_modifier(Modifier::DIM),
                )));
            }
            ChatEntryKind::Actor { source, text } => {
                render_truncated_lines(lines, &format!("  [{source}] {text}"), 80);
            }
            ChatEntryKind::Compaction { .. } => {
                lines.push(Line::from(Span::styled(
                    "  [compaction summary]",
                    Style::default().add_modifier(Modifier::DIM),
                )));
            }
            ChatEntryKind::Transient(_) => {
                // Skip ephemeral entries
            }
        }
    }
}

/// Renders text that may contain newlines as multiple inspector lines.
/// Each resulting line is truncated to `max_chars` characters.
fn render_truncated_lines(lines: &mut Vec<Line<'static>>, text: &str, max_chars: usize) {
    for raw_line in text.lines() {
        let truncated: String = raw_line.chars().take(max_chars).collect();
        lines.push(Line::from(Span::styled(
            truncated,
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
}

/// Renders the sticky inspector popup overlay.
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
        .map_or_else(|| "Unknown".to_owned(), |s| format!("{s:?}"));

    // Check if this node has an associated session.
    let has_session = workflow
        .node_sessions
        .get(node_name)
        .and_then(|id| state.session.get(id))
        .is_some();

    let lines = build_inspector_lines(node_name, node_state, &status, state);

    // Compute popup dimensions — expand for session history.
    let content_height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let popup_width = if has_session {
        (area.width * 70 / 100).clamp(40, 80)
    } else {
        (area.width * 50 / 100).clamp(30, 60)
    };
    let desired_height = content_height.saturating_add(2); // +2 for borders
    let max_height = if has_session {
        area.height * 80 / 100
    } else {
        area.height / 2
    };
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
    use nullslop_workflow::port::{PortValue, ScalarValue};
    match value {
        PortValue::Single(ScalarValue::Text(s)) => {
            if s.len() > 80 {
                let truncated: String = s.chars().take(77).collect();
                format!("{truncated}...")
            } else {
                s.clone()
            }
        }
        PortValue::Single(ScalarValue::Number(n)) => {
            let s = format!("{n}");
            if s.len() > 80 {
                let truncated: String = s.chars().take(77).collect();
                format!("{truncated}...")
            } else {
                s
            }
        }
        PortValue::Single(ScalarValue::Boolean(b)) => b.to_string(),
        PortValue::Single(ScalarValue::Json(v)) => {
            let s = serde_json::to_string(v).unwrap_or_else(|_| format!("{v}"));
            if s.len() > 200 {
                let truncated: String = s.chars().take(197).collect();
                format!("{truncated}...")
            } else {
                s
            }
        }
        PortValue::Vector(items) => {
            let s = format!("[{} items]", items.len());
            s
        }
        PortValue::Map(entries) => {
            let s = format!("{{{} entries}}", entries.len());
            s
        }
    }
}
