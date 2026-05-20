//! Google Gemini API provider implementation.
//!
//! Implements streaming chat completions for Gemini models via Google's
//! Generative Language API. Key differences from OpenAI-compatible:
//!
//! - API key passed as query parameter (not header)
//! - Messages use `contents` array with `role` ("user"/"model") and `parts`
//! - System prompt as top-level `systemInstruction` field
//! - Tool definitions use `functionDeclarations` array
//! - Response has `candidates[0].content.parts[0].text` for text
//! - SSE format is the same (`data: {...}\n\n`)

mod factory;
mod models;
mod request;
mod response;
mod service;

pub use factory::GoogleFactory;
pub use service::GoogleService;
