//! Edit bench tasks — model must make precise edits to existing files.
//!
//! Tasks provide the model with `read` + `write` tools only (no `bash`).
//! Verification uses snapshot comparison against expected output files.

use crate::task::BenchTask;

mod edit_config_value;
mod edit_typo_large_text;

/// Returns all edit bench tasks.
pub fn tasks() -> Vec<BenchTask> {
    vec![
        edit_typo_large_text::task(),
        edit_config_value::task(),
    ]
}
