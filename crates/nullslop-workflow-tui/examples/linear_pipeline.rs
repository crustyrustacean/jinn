//! Linear pipeline: source → transform → sink.
//!
//! Three connected nodes forming a linear pipeline. Shows connections
//! between ports, node layout in columns, and type labels.
//!
//! ```sh
//! cargo run -p nullslop-workflow-tui --example linear-pipeline
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
use ratatui::widgets::Widget;

#[expect(clippy::expect_used, reason = "example code")]
fn main() {
    let mut terminal = common::setup_terminal();

    let graph = {
        let mut b = WorkflowGraphBuilder::new();
        b.add_node(
            "source".to_owned(),
            common::make_node("source", vec![], vec![PortDef::string("text")]),
        );
        b.add_node(
            "transform".to_owned(),
            common::make_node(
                "transform",
                vec![PortDef::string("input")],
                vec![PortDef::string("output")],
            ),
        );
        b.add_node(
            "sink".to_owned(),
            common::make_node("sink", vec![PortDef::string("text")], vec![]),
        );
        b.connect("source", "text", "transform", "input")
            .expect("connect");
        b.connect("transform", "output", "sink", "text")
            .expect("connect");
        b.build().expect("graph should build")
    };

    // Mix of statuses to see different indicators.
    let execution = WorkflowExecution::new(graph);
    execution.set_status("source", NodeStatus::Completed);
    execution.set_status("transform", NodeStatus::Running);
    // sink stays Pending (default)
    let snapshot = execution.snapshot();
    let viewport = ViewportState::new();

    terminal
        .draw(|f| {
            let widget = WorkflowWidget::new(&snapshot, &viewport, 0);
            widget.render(f.area(), f.buffer_mut());
        })
        .expect("draw failed");

    thread::sleep(Duration::from_secs(5));
    common::restore_terminal(&mut terminal);
}
