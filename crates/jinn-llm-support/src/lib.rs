//! Kernel-free LLM support vocabulary.
//!
//! This crate owns the pure machinery that sits between the chat-entry
//! vocabulary and the LLM wire: token estimation, tool-prompt blocks,
//! chat-entry → `LlmMessage` conversion, and image attachment conversion.
//!
//! It deliberately depends on neither the domain kernel nor the slice
//! contracts. That placement is what makes it reachable from both — every
//! consumer of these modules already depends on `jinn-domain`, so these
//! modules can only live in a crate that nothing's dependency path reaches
//! from.

#![forbid(unsafe_code)]

pub mod entries_to_messages;
pub mod image_convert;
pub mod token_estimator;
pub mod tool_prompt;

#[cfg(test)]
mod entries_to_messages_tests;
#[cfg(test)]
mod token_estimator_tests;
