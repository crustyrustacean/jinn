//! Built-in node types for common workflow patterns.
//!
//! - [`CodeNode`] — wraps an async closure for quick custom logic.
//! - [`DelayNode`] — sleeps for a configured duration, then passes inputs through.

pub mod code_node;
pub mod delay_node;

pub use code_node::CodeNode;
pub use delay_node::DelayNode;
