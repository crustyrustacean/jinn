//! Bench task definitions.
//!
//! Each task is a [`BenchTask`] with a prompt, optional fixtures, tool
//! configuration, and a verification function. Tasks are organized into
//! category submodules:
//!
//! - **edit**: Model makes precise edits to existing files (no `bash`).
//! - **one_shot**: Single message, model produces output from scratch.
//! - **fix_code**: Model receives broken code and must fix it.
//! - **redirect**: Multi-turn, model is asked to do X, then told to do Y instead.

pub mod checks;
pub mod edit;
pub mod fix_code;
pub mod one_shot;
pub mod redirect;

use crate::task::BenchTask;

/// Returns all benchmark tasks.
pub fn bench_tasks() -> Vec<BenchTask> {
    let mut tasks = Vec::new();
    tasks.extend(edit::tasks());
    tasks.extend(one_shot::tasks());
    tasks.extend(fix_code::tasks());
    tasks.extend(redirect::tasks());
    tasks
}
