//! Domain-specific workflow nodes.
//!
//! Nodes that require access to the domain layer (LLM via actor bus, etc.).

pub mod llm;

pub use llm::LlmNode;
