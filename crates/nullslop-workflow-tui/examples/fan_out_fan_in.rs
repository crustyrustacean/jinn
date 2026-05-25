//! Fan-out and fan-in: source fans out to two transforms, which both feed into a merge sink.
//!
//! Shows fan-out (one output → two inputs), fan-in (two outputs → one node),
//! multiple connections, and a node with multiple input ports of different types.
//!
//! ```sh
//! cargo run -p nullslop-workflow-tui --example fan-out-fan-in
//! ```

#[path = "utils/mod.rs"]
mod common;

use std::thread;
use std::time::Duration;

use nullslop_workflow::engine::NodeStatus;
use nullslop_workflow::execution::WorkflowExecution;
use nullslop_workflow::graph::WorkflowGraphBuilder;
use nullslop_workflow::port::PortDef;
use nullslop_workflow_tui::viewport::ViewportState;
use nullslop_workflow_tui::widget::WorkflowWidget;
use ratatui::style::Color;
use ratatui::widgets::Widget;

#[expect(clippy::expect_used, reason = "example code")]
fn main() {
    let mut terminal = common::setup_terminal();

    let graph = {
        let mut b = WorkflowGraphBuilder::new();
        b.add_node(
            "source".to_owned(),
            common::make_node("source", vec![], vec![PortDef::text("text")]),
        );
        b.add_node(
            "uppercase".to_owned(),
            common::make_node(
                "uppercase",
                vec![PortDef::text("input")],
                vec![PortDef::text("result")],
            ),
        );
        b.add_node(
            "reverse".to_owned(),
            common::make_node(
                "reverse",
                vec![PortDef::text("input")],
                vec![PortDef::text("result")],
            ),
        );
        // Merge sink with two String input ports.
        b.add_node(
            "merge".to_owned(),
            common::make_node(
                "merge",
                vec![
                    PortDef::text("upper_result"),
                    PortDef::text("reverse_result"),
                ],
                vec![],
            ),
        );

        // Fan-out: source → both transforms
        b.connect("source", "text", "uppercase", "input")
            .expect("connect");
        b.connect("source", "text", "reverse", "input")
            .expect("connect");
        // Fan-in: both transforms → merge (different port types!)
        b.connect("uppercase", "result", "merge", "upper_result")
            .expect("connect");
        b.connect("reverse", "result", "merge", "reverse_result")
            .expect("connect");
        b.build().expect("graph should build")
    };

    let execution = WorkflowExecution::new(graph);
    execution.set_status("source", NodeStatus::Completed);
    execution.set_status("uppercase", NodeStatus::Completed);
    execution.set_status("reverse", NodeStatus::Running);
    // merge stays Pending
    let snapshot = execution.snapshot();
    let viewport = ViewportState::new();

    terminal
        .draw(|f| {
            let widget = WorkflowWidget::new(&snapshot, &viewport, 0, Color::Cyan);
            widget.render(f.area(), f.buffer_mut());
        })
        .expect("draw failed");

    thread::sleep(Duration::from_secs(5));
    common::restore_terminal(&mut terminal);
}
