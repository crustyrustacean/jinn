//! Interactive viewport with panning and node selection.
//!
//! A 6-node linear pipeline that's wider than a typical terminal.
//! Use arrow keys to pan, Tab/Shift+Tab to cycle node selection, 'q' to quit.
//!
//! ```sh
//! cargo run -p jinn-workflow-tui --example viewport-pan
//! ```

#[path = "utils/mod.rs"]
mod common;

use std::time::Duration;

use jinn_workflow::engine::NodeStatus;
use jinn_workflow::execution::WorkflowExecution;
use jinn_workflow::graph::WorkflowGraphBuilder;
use jinn_workflow::port::PortDef;
use jinn_workflow_tui::layout;
use jinn_workflow_tui::viewport::ViewportState;
use jinn_workflow_tui::widget::WorkflowWidget;
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Paragraph, Widget};

/// Builds a 6-node linear pipeline graph: load_data → parse → validate → enrich → transform → save.
fn build_pipeline() -> WorkflowExecution {
    let graph = {
        let mut b = WorkflowGraphBuilder::new();
        b.add_node(
            "load_data".to_owned(),
            common::make_node("load_data", vec![], vec![PortDef::text("raw")]),
        );
        b.add_node(
            "parse".to_owned(),
            common::make_node(
                "parse",
                vec![PortDef::text("raw")],
                vec![PortDef::json("records")],
            ),
        );
        b.add_node(
            "validate".to_owned(),
            common::make_node(
                "validate",
                vec![PortDef::json("records")],
                vec![PortDef::json("valid")],
            ),
        );
        b.add_node(
            "enrich".to_owned(),
            common::make_node(
                "enrich",
                vec![PortDef::json("valid")],
                vec![PortDef::json("enriched")],
            ),
        );
        b.add_node(
            "transform".to_owned(),
            common::make_node(
                "transform",
                vec![PortDef::json("enriched")],
                vec![PortDef::text("output")],
            ),
        );
        b.add_node(
            "save".to_owned(),
            common::make_node("save", vec![PortDef::text("output")], vec![]),
        );

        b.connect("load_data", "raw", "parse", "raw").unwrap();
        b.connect("parse", "records", "validate", "records")
            .unwrap();
        b.connect("validate", "valid", "enrich", "valid").unwrap();
        b.connect("enrich", "enriched", "transform", "enriched")
            .unwrap();
        b.connect("transform", "output", "save", "output").unwrap();
        b.build().unwrap()
    };

    let execution = WorkflowExecution::new(graph);
    execution.set_status("load_data", NodeStatus::Completed);
    execution.set_status("parse", NodeStatus::Completed);
    execution.set_status("validate", NodeStatus::Completed);
    execution.set_status("enrich", NodeStatus::Running);
    // transform and save stay Pending
    execution
}

#[expect(clippy::expect_used, reason = "example code")]
fn main() {
    let mut terminal = common::setup_terminal();

    let execution = build_pipeline();
    let node_names: Vec<String> = execution
        .snapshot()
        .structure()
        .node_names()
        .map(std::borrow::ToOwned::to_owned)
        .collect();

    let the_layout = layout::compute(&execution.snapshot());
    let content_size = the_layout.content_size();

    let mut viewport = ViewportState::new();
    let mut tick: u8 = 0;
    let mut viewport_dims = (80u16, 24u16);

    loop {
        let snapshot = execution.snapshot();
        terminal
            .draw(|f| {
                let help_height = 1u16;
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

                viewport_dims = (main_area.width, main_area.height);

                let help = format!(
                    " ←↑↓→ move │ Tab/Shift+Tab select │ q quit │ selected: {} │ offset: ({}, {})",
                    viewport.selected_node().unwrap_or("none"),
                    viewport.offset_x,
                    viewport.offset_y,
                );
                f.render_widget(
                    Paragraph::new(help).style(Style::default().fg(Color::DarkGray)),
                    help_area,
                );
            })
            .expect("draw failed");

        tick = tick.wrapping_add(1);

        if ratatui::crossterm::event::poll(Duration::from_millis(80)).expect("poll")
            && let Event::Key(key) = ratatui::crossterm::event::read().expect("read")
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Left => viewport.translate(3, 0, content_size, viewport_dims),
                KeyCode::Right => viewport.translate(-3, 0, content_size, viewport_dims),
                KeyCode::Up => viewport.translate(0, 1, content_size, viewport_dims),
                KeyCode::Down => viewport.translate(0, -1, content_size, viewport_dims),
                KeyCode::Tab => viewport.select_next(&node_names),
                KeyCode::BackTab => viewport.select_prev(&node_names),
                _ => {}
            }
        }
    }

    common::restore_terminal(&mut terminal);
}
