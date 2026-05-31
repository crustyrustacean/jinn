//! Workflow graph visualization for ratatui.
//!
//! Renders a [`WorkflowGraph`](jinn_workflow::graph::WorkflowGraph) as an interactive
//! terminal widget with rounded boxes, typed ports, status indicators, and
//! connection lines. Supports viewport panning and node selection.
//!
//! # Crate structure
//!
//! - [`node`] - Visual node rendering and dimension calculation.
//! - [`port`] - Port type color legend.
//! - [`status`] - Status indicator symbols and colors.

pub mod connection;
pub mod layout;
pub mod node;
pub mod port;
pub mod status;
pub mod viewport;
pub mod widget;
