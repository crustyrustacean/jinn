//! jinn-mcp — MCP client support for jinn.
//!
//! Wraps the `rmcp` crate to connect to an MCP server over a stdio child
//! process, list its tools, and invoke them. This crate is transport-level:
//! it knows nothing about the actor system or `AppState`. The
//! `jinn-domain::feat::mcp_actor` module drives it from inside the actor
//! system.

pub mod client;
pub mod tool_mapping;

pub use client::{McpClient, McpClientError, ServerCommand};

// Re-export rmcp model types so downstream crates (jinn-domain) can pattern-
// match on tool results without taking a direct rmcp dependency.
pub use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool};

// Test-only constructors for rmcp types that are `#[non_exhaustive]` and so
// cannot be built with a struct literal from outside the rmcp crate.
// Gated behind the `testkit` feature so production builds never pull these in.
#[cfg(feature = "testkit")]
pub mod testkit;

// A reusable stub MCP server for downstream integration tests.
// Gated behind `server-testkit` so production builds never pull in rmcp's
// server implementation.
#[cfg(feature = "server-testkit")]
pub mod server_testkit;
