//! Live execution: runs a real async workflow using DelayNode and shows the final state.
//!
//! Uses the actual workflow engine with real async delays. The display shows
//! nodes in a "mid-execution" state while the engine runs in the background,
//! then updates to show the final completed state.
//!
//! ```sh
//! cargo run -p nullslop-workflow-tui --example live-execution
//! ```

#[path = "utils/mod.rs"]
mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use nullslop_workflow::engine::{self, NodeStatus};
use nullslop_workflow::graph::WorkflowGraphBuilder;
use nullslop_workflow::node::NodeContext;
use nullslop_workflow::nodes::delay_node::DelayNode;
use nullslop_workflow::port::PortDef;
use nullslop_workflow_tui::viewport::ViewportState;
use nullslop_workflow_tui::widget::WorkflowWidget;
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Paragraph, Widget};

/// Minimal context for the engine.
struct Ctx;
impl NodeContext for Ctx {}

fn build_graph() -> nullslop_workflow::graph::WorkflowGraph {
    let mut b = WorkflowGraphBuilder::new();
    // First node: source (no inputs, one output) — use make_node since DelayNode mirrors ports.
    b.add_node(
        "fast".to_owned(),
        common::make_node("fast", vec![], vec![PortDef::string("out")]),
    );
    // Middle and last: passthrough delay nodes (input "in" -> output "in").
    b.add_node(
        "medium".to_owned(),
        Box::new(DelayNode::passthrough(Duration::from_millis(1500))),
    );
    b.add_node(
        "slow".to_owned(),
        Box::new(DelayNode::passthrough(Duration::from_millis(3000))),
    );
    b.connect("fast", "out", "medium", "in").unwrap();
    b.connect("medium", "in", "slow", "in").unwrap();
    b.build().unwrap()
}

#[tokio::main]
async fn main() {
    let mut terminal = common::setup_terminal();

    // Build a separate graph for rendering (the engine consumes its graph).
    let render_graph = build_graph();
    let node_names: Vec<String> = render_graph.node_names().map(|n| n.to_owned()).collect();

    // Spawn the engine in a background task with its own graph.
    let engine_handle = tokio::spawn(async { engine::execute(build_graph(), Arc::new(Ctx)).await });
    let viewport = ViewportState::new();
    let mut tick: u8 = 0;

    // Show "running" state while the engine works.
    loop {
        terminal
            .draw(|f| {
                let help_height = 2u16;
                let main_area = Rect {
                    x: f.area().x,
                    y: f.area().y,
                    width: f.area().width,
                    height: f.area().height.saturating_sub(help_height),
                };
                let help_area = Rect {
                    x: f.area().x,
                    y: f.area().y + f.area().height.saturating_sub(help_height),
                    width: f.area().width,
                    height: help_height,
                };

                // Show a "mid-execution" snapshot.
                let statuses = HashMap::from([
                    ("fast".to_owned(), NodeStatus::Completed),
                    ("medium".to_owned(), NodeStatus::Running),
                    ("slow".to_owned(), NodeStatus::Pending),
                ]);

                let widget = WorkflowWidget::new(&render_graph, &statuses, &viewport, tick);
                widget.render(main_area, f.buffer_mut());

                f.render_widget(
                    Paragraph::new(" ⏳ Engine running with real async DelayNodes... (q to quit)")
                        .style(Style::default().fg(Color::Yellow)),
                    help_area,
                );
            })
            .expect("draw failed");

        tick = tick.wrapping_add(1);

        if ratatui::crossterm::event::poll(Duration::from_millis(80)).expect("poll") {
            if let Event::Key(key) = ratatui::crossterm::event::read().expect("read") {
                if key.kind == KeyEventKind::Press && key.code == KeyCode::Char('q') {
                    engine_handle.abort();
                    common::restore_terminal(&mut terminal);
                    return;
                }
            }
        }

        if engine_handle.is_finished() {
            break;
        }
    }

    // Show final state from engine result.
    let result = engine_handle.await.expect("task join");
    let final_statuses = match result {
        Ok(res) => res.statuses,
        Err(_) => node_names
            .iter()
            .map(|n| (n.clone(), NodeStatus::Failed))
            .collect(),
    };

    terminal
        .draw(|f| {
            let help_height = 2u16;
            let main_area = Rect {
                x: f.area().x,
                y: f.area().y,
                width: f.area().width,
                height: f.area().height.saturating_sub(help_height),
            };
            let help_area = Rect {
                x: f.area().x,
                y: f.area().y + f.area().height.saturating_sub(help_height),
                width: f.area().width,
                height: help_height,
            };

            let widget = WorkflowWidget::new(&render_graph, &final_statuses, &viewport, tick);
            widget.render(main_area, f.buffer_mut());

            f.render_widget(
                Paragraph::new(" ✅ Workflow complete! All nodes succeeded. (q to quit)")
                    .style(Style::default().fg(Color::Green)),
                help_area,
            );
        })
        .expect("draw failed");

    // Wait for user to quit.
    loop {
        if ratatui::crossterm::event::poll(Duration::from_millis(100)).expect("poll") {
            if let Event::Key(key) = ratatui::crossterm::event::read().expect("read") {
                if key.kind == KeyEventKind::Press && key.code == KeyCode::Char('q') {
                    break;
                }
            }
        }
    }

    common::restore_terminal(&mut terminal);
}
