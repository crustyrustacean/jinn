//! Shared helpers for workflow-tui examples.
//!
//! Provides terminal setup/teardown and a `make_node()` function for defining
//! placeholder [`WorkflowNode`]s with typed ports.

#![allow(
    dead_code,
    reason = "shared example utilities - not all examples use every function"
)]

use std::io::Stdout;

use nullslop_workflow::node::{NodeContext, NodeError, WorkflowNode};
use nullslop_workflow::port::{PortDef, PortValues};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self};
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// Terminal type alias for example helpers.
pub type Term = Terminal<CrosstermBackend<Stdout>>;

/// Sets up the terminal for TUI rendering.
#[expect(clippy::expect_used, reason = "example code")]
pub fn setup_terminal() -> Term {
    enable_raw_mode().expect("failed to enable raw mode");
    crossterm::execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        event::EnableMouseCapture
    )
    .expect("failed to enter alternate screen");
    let backend = CrosstermBackend::new(std::io::stdout());
    Terminal::new(backend).expect("failed to create terminal")
}

/// Restores the terminal to its original state.
#[expect(clippy::expect_used, reason = "example code")]
pub fn restore_terminal(terminal: &mut Term) {
    disable_raw_mode().expect("failed to disable raw mode");
    crossterm::execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        event::DisableMouseCapture
    )
    .expect("failed to leave alternate screen");
}

/// Creates a boxed placeholder [`WorkflowNode`] with the given name, inputs, and outputs.
///
/// The node's `execute` method returns empty `PortValues`.
/// Intended for visual examples where you want to see the node rendering without real logic.
pub fn make_node(
    name: &'static str,
    inputs: Vec<PortDef>,
    outputs: Vec<PortDef>,
) -> Box<dyn WorkflowNode> {
    struct N {
        name: &'static str,
        inputs: Vec<PortDef>,
        outputs: Vec<PortDef>,
    }

    #[async_trait::async_trait]
    impl WorkflowNode for N {
        fn name(&self) -> &str {
            self.name
        }
        fn input_ports(&self) -> Vec<PortDef> {
            self.inputs.clone()
        }
        fn output_ports(&self) -> Vec<PortDef> {
            self.outputs.clone()
        }
        async fn execute(
            &self,
            _inputs: PortValues,
            _ctx: &dyn NodeContext,
        ) -> Result<PortValues, error_stack::Report<NodeError>> {
            Ok(PortValues::new())
        }
        fn clone_box(&self) -> Box<dyn WorkflowNode> {
            Box::new(N {
                name: self.name,
                inputs: self.inputs.clone(),
                outputs: self.outputs.clone(),
            })
        }
    }

    Box::new(N {
        name,
        inputs,
        outputs,
    })
}
