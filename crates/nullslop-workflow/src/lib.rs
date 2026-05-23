//! Programmable workflow engine — DAG-based dataflow with typed ports.
//!
//! This crate provides the core types and execution engine for building
//! workflow graphs where data flows from node to node through named,
//! typed ports. Nodes execute asynchronously and independent branches
//! run concurrently.
//!
//! # Core concepts
//!
//! - **Node** — a unit of computation that declares input/output ports.
//! - **Port** — a named, typed endpoint on a node (`PortDef`).
//! - **Edge** — a connection from one node's output port to another's input port.
//! - **Graph** — a validated DAG of nodes and edges (`WorkflowGraph`).
//! - **Engine** — executes a graph, pushing data from sources to sinks.

pub mod engine;
pub mod graph;
pub mod node;
pub mod port;
