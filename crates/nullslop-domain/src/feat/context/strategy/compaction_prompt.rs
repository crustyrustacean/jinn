//! Bundled fallback for the compaction prompt.
//!
//! The primary source for the compaction prompt is the [`PromptTemplateStore`]
//! (populated by the prompt scan actor from system and user directories).
//! This constant serves as the final fallback when no template named
//! "compaction" is found in the store.
//!
//! [`PromptTemplateStore`]: crate::feat::context::prompt_template::PromptTemplateStore

/// Bundled default compaction prompt (from `prompts/compaction.md`).
///
/// Used as a fallback when the prompt template store does not contain
/// a template named "compaction".
pub const DEFAULT_COMPACTION_PROMPT: &str = include_str!("../../../../../../prompts/compaction.md");
