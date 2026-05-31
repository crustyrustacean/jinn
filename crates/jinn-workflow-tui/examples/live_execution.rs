//! Live execution: runs a real async workflow using DelayNode and shows the final state.
//!
//! Uses the actual workflow engine with real async delays. The display shows
//! nodes in a "mid-execution" state while the engine runs in the background,
//! then updates to show the final completed state.
//!
//! ```sh
//! cargo run -p jinn-workflow-tui --example live-execution
//! ```

#[path = "utils/mod.rs"]
mod common;

use std::sync::Arc;
use std::time::Duration;

use jinn_workflow::engine;
use jinn_workflow::execution::WorkflowExecution;
use jinn_workflow::graph::WorkflowGraphBuilder;
use jinn_workflow::node::NodeContext;
use jinn_workflow::node::delay::DelayNode;
use jinn_workflow::port::PortDef;
use jinn_workflow_tui::viewport::ViewportState;
use jinn_workflow_tui::widget::WorkflowWidget;
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Paragraph, Widget};

/// Minimal context for the engine.
struct Ctx;
impl NodeContext for Ctx {}

/// Builds the workflow graph for the live execution example.
fn build_graph() -> jinn_workflow::graph::WorkflowGraph {
    let mut b = WorkflowGraphBuilder::new();
    // First node: source (no inputs, one output) - use make_node since DelayNode mirrors ports.
    b.add_node(
        "fast".to_owned(),
        common::make_node("fast", vec![], vec![PortDef::text("out")]),
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
#[expect(clippy::expect_used, reason = "example code")]
async fn main() {
    let mut terminal = common::setup_terminal();

    // Spawn the engine in a background task.
    let execution = Arc::new(WorkflowExecution::new(build_graph()));
    let engine_exec = execution.clone();
    let engine_handle =
        tokio::spawn(async move { engine::execute(engine_exec, Arc::new(Ctx)).await });
    let viewport = ViewportState::new();
    let mut tick: u8 = 0;

    // Show live state while the engine works.
    loop {
        let snapshot = execution.snapshot();
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

                let widget = WorkflowWidget::new(&snapshot, &viewport, tick, Color::Cyan);
                widget.render(main_area, f.buffer_mut());

                f.render_widget(
                    Paragraph::new(" ⏳ Engine running with real async DelayNodes... (q to quit)")
                        .style(Style::default().fg(Color::Yellow)),
                    help_area,
                );
            })
            .expect("draw failed");

        tick = tick.wrapping_add(1);

        if ratatui::crossterm::event::poll(Duration::from_millis(80)).expect("poll")
            && let Event::Key(key) = ratatui::crossterm::event::read().expect("read")
            && key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('q')
        {
            engine_handle.abort();
            common::restore_terminal(&mut terminal);
            return;
        }

        if engine_handle.is_finished() {
            break;
        }
    }

    // Show final state from the execution snapshot.
    let result = engine_handle.await.expect("task join");
    let snapshot = execution.snapshot();

    let status_msg = match result {
        Ok(_) => Paragraph::new(" ✅ Workflow complete! All nodes succeeded. (q to quit)")
            .style(Style::default().fg(Color::Green)),
        Err(_) => Paragraph::new(" ❌ Workflow failed. (q to quit)")
            .style(Style::default().fg(Color::Red)),
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

            let widget = WorkflowWidget::new(&snapshot, &viewport, tick, Color::Cyan);
            widget.render(main_area, f.buffer_mut());

            f.render_widget(status_msg, help_area);
        })
        .expect("draw failed");

    // Wait for user to quit.
    loop {
        if ratatui::crossterm::event::poll(Duration::from_millis(100)).expect("poll")
            && let Event::Key(key) = ratatui::crossterm::event::read().expect("read")
            && key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('q')
        {
            break;
        }
    }

    common::restore_terminal(&mut terminal);
}
