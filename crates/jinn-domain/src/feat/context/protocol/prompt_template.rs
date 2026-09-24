//! Prompt template domain types.
//!
//! The [`PromptTemplate`] noun is owned by the session-init slice's msg
//! crate (it crosses the prompt-scan boundary); re-exported here so the
//! kernel-side store/loader keep one import path.
pub use jinn_session_init_msg::PromptTemplate;
