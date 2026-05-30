//! OpenAI-compatible provider implementation.
//!
//! Implements streaming chat completions and model listing for any
//! provider that uses the OpenAI chat completions format. Each backend
//! is configured via a [`ProviderConfig`] that provides base URL, custom
//! headers, and endpoint paths.

mod factory;
mod models;
mod provider_config;
mod request;
mod response;
mod service;
pub mod sse;

pub use factory::OpenAiCompatibleFactory;
pub use provider_config::ProviderConfig;
pub use service::OpenAiCompatibleService;
