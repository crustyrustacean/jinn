//! LLM service trait and error types.
//!
//! Core types are defined in the `jinn-provider` crate and re-exported
//! here for convenience.

pub use jinn_provider::{ChatStream, LlmService, LlmServiceError, LlmServiceFactory, ToolStream};
