//! Minimal example: a single source node.
//!
//! Renders one node with no inputs and one String output port.
//! Shows the basic node box with rounded corners, status indicator, and port.
//!
//! ```sh
//! cargo run -p jinn-workflow-tui --example single-node
//! ```

#[path = "utils/mod.rs"]
mod common;

use std::thread;
use std::time::Duration;

use jinn_workflow::execution::WorkflowExecution;
use jinn_workflow::graph::WorkflowGraphBuilder;
use jinn_workflow::port::PortDef;
use jinn_workflow_tui::viewport::ViewportState;
use jinn_workflow_tui::widget::WorkflowWidget;
use ratatui::style::Color;
use ratatui::widgets::Widget;

#[expect(clippy::expect_used, reason = "example code")]
fn main() {
    let mut terminal = common::setup_terminal();

    let graph = {
        let mut b = WorkflowGraphBuilder::new();
        b.add_node(
            "source".to_owned(),
            common::make_node("source", vec![], vec![PortDef::text("output")]),
        );
        b.build().expect("graph should build")
    };

    let execution = WorkflowExecution::new(graph);
    let snapshot = execution.snapshot();
    let viewport = ViewportState::new();

    terminal
        .draw(|f| {
            let widget = WorkflowWidget::new(&snapshot, &viewport, 0, Color::Cyan);
            widget.render(f.area(), f.buffer_mut());
        })
        .expect("draw failed");

    // Hold the display for 3 seconds, then quit.
    thread::sleep(Duration::from_secs(3));
    common::restore_terminal(&mut terminal);
}
