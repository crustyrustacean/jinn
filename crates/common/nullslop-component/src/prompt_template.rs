//! Prompt templates — reusable prompts loaded from `~/.config/nullslop/prompts/`.
//!
//! Re-exports types from [`nullslop_prompt_template`]. All logic lives in the
//! standalone crate; this module is a convenience re-export point.

// Re-export from the standalone crate.
pub use nullslop_prompt_template::{
    PromptTemplateParseError, PromptTemplateStore, PromptTemplateStoreError, expand_tokens,
    prompts_dir,
};
// Re-export PromptTemplate from protocol (also available through the standalone crate).
pub use nullslop_protocol::PromptTemplate;
