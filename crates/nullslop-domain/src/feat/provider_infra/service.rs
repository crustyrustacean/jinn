//! LLM service trait and error types.
//!
//! Core types are defined in the `nullslop-provider` crate and re-exported
//! here for convenience.

pub use nullslop_provider::{
    ChatStream, LlmService, LlmServiceError, LlmServiceFactory, ToolStream,
};
