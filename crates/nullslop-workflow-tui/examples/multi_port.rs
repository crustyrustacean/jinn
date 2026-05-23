//! Multi-port node with mixed types (String + Json).
//!
//! A single node with 2 input ports (String prompt, Json config) and
//! 2 output ports (String response, Json usage). Shows port type coloring
//! (green for String, blue for Json), the gap row between inputs and outputs,
//! and the running spinner animation.
//!
//! ```sh
//! cargo run -p nullslop-workflow-tui --example multi-port
//! ```

#[path = "utils/mod.rs"]
mod common;

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use nullslop_workflow::engine::NodeStatus;
use nullslop_workflow::graph::WorkflowGraphBuilder;
use nullslop_workflow::port::PortDef;
use nullslop_workflow_tui::viewport::ViewportState;
use nullslop_workflow_tui::widget::WorkflowWidget;
use ratatui::widgets::Widget;

fn main() {
    let mut terminal = common::setup_terminal();

    let graph = {
        let mut b = WorkflowGraphBuilder::new();
        b.add_node(
            "llm-call".to_owned(),
            common::make_node(
                "llm-call",
                vec![PortDef::string("prompt"), PortDef::json("config")],
                vec![PortDef::string("response"), PortDef::json("usage")],
            ),
        );
        b.build().expect("graph should build")
    };

    let statuses = HashMap::from([("llm-call".to_owned(), NodeStatus::Running)]);
    let viewport = ViewportState::new();

    // Animate the spinner for a few frames then hold.
    for tick in 0..30u8 {
        terminal
            .draw(|f| {
                let widget = WorkflowWidget::new(&graph, &statuses, &viewport, tick);
                widget.render(f.area(), f.buffer_mut());
            })
            .expect("draw failed");
        thread::sleep(Duration::from_millis(100));
    }

    common::restore_terminal(&mut terminal);
}
