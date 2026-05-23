//! Interactive viewport with panning and node selection.
//!
//! A 6-node linear pipeline that's wider than a typical terminal.
//! Use arrow keys to pan, Tab/Shift+Tab to cycle node selection, 'q' to quit.
//!
//! ```sh
//! cargo run -p nullslop-workflow-tui --example viewport-pan
//! ```

#[path = "utils/mod.rs"]
mod common;

use std::collections::HashMap;
use std::time::Duration;

use nullslop_workflow::engine::NodeStatus;
use nullslop_workflow::graph::WorkflowGraphBuilder;
use nullslop_workflow::port::PortDef;
use nullslop_workflow_tui::layout;
use nullslop_workflow_tui::viewport::ViewportState;
use nullslop_workflow_tui::widget::WorkflowWidget;
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Paragraph, Widget};

fn main() {
    let mut terminal = common::setup_terminal();

    // Build: load_data → parse → validate → enrich → transform → output
    let graph = {
        let mut b = WorkflowGraphBuilder::new();
        b.add_node(
            "load_data".to_owned(),
            common::make_node("load_data", vec![], vec![PortDef::string("raw")]),
        );
        b.add_node(
            "parse".to_owned(),
            common::make_node(
                "parse",
                vec![PortDef::string("raw")],
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
                vec![PortDef::string("output")],
            ),
        );
        b.add_node(
            "save".to_owned(),
            common::make_node("save", vec![PortDef::string("output")], vec![]),
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

    let node_names: Vec<String> = graph.node_names().map(|n| n.to_owned()).collect();

    let statuses = HashMap::from([
        ("load_data".to_owned(), NodeStatus::Completed),
        ("parse".to_owned(), NodeStatus::Completed),
        ("validate".to_owned(), NodeStatus::Completed),
        ("enrich".to_owned(), NodeStatus::Running),
        ("transform".to_owned(), NodeStatus::Pending),
        ("save".to_owned(), NodeStatus::Pending),
    ]);

    let the_layout = layout::compute(&graph, &statuses);
    let content_size = the_layout.content_size();

    let mut viewport = ViewportState::new();
    let mut tick: u8 = 0;
    let mut viewport_dims = (80u16, 24u16);

    loop {
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

                let widget = WorkflowWidget::new(&graph, &statuses, &viewport, tick);
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

        if ratatui::crossterm::event::poll(Duration::from_millis(80)).expect("poll") {
            if let Event::Key(key) = ratatui::crossterm::event::read().expect("read") {
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
    }

    common::restore_terminal(&mut terminal);
}
