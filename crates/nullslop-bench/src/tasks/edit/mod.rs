//! Edit bench tasks — model must make precise edits to existing files.
//!
//! Tasks provide the model with `read` + `write` tools only (no `bash`).
//! Verification uses snapshot comparison against expected output files.

use crate::task::BenchTask;

mod edit_config_value;
mod edit_duplicate_sections;
mod edit_insert_function;
mod edit_json_array;
mod edit_large_replace_small_file;
mod edit_rename_all;
mod edit_typo_large_text;

/// Returns all edit bench tasks.
pub fn tasks() -> Vec<BenchTask> {
    vec![
        edit_typo_large_text::task(),
        edit_config_value::task(),
        edit_json_array::task(),
        edit_duplicate_sections::task(),
        edit_insert_function::task(),
        edit_large_replace_small_file::task(),
        edit_rename_all::task(),
    ]
}
