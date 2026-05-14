//! Anthropic Messages API provider implementation.
//!
//! Implements streaming chat completions for Claude models via Anthropic's
//! Messages API. Key differences from OpenAI-compatible:
//!
//! - System prompt is a top-level field, not in the messages array
//! - Auth via `x-api-key` header (not `Authorization: Bearer`)
//! - Requires `anthropic-version` header
//! - Tool definitions use `input_schema` (not `parameters`)
//! - SSE events use `content_block_start/delta/stop` + `message_delta`
//! - Tool calls complete on `content_block_stop` (not `finish_reason`)
//! - Empty tool arguments default to `"{}"` (not empty string)

mod factory;
mod models;
mod request;
mod response;
mod service;

pub use factory::AnthropicFactory;
pub use service::AnthropicService;
