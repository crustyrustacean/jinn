//! Redirect bench tasks — multi-turn, model is asked to do X, then told to do Y instead.

use crate::task::BenchTask;

mod redirect_change_color;
mod redirect_refactor_function;
mod redirect_switch_language;

/// Returns all redirect bench tasks.
pub fn tasks() -> Vec<BenchTask> {
    vec![
        redirect_change_color::task(),
        redirect_refactor_function::task(),
        redirect_switch_language::task(),
    ]
}
