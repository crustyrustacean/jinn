//! Fix-code bench tasks — model receives broken code and must fix it.

use crate::task::BenchTask;

mod fix_logic_fizzbuzz;
mod fix_logic_sort;
mod fix_syntax_broken_python;
mod fix_syntax_broken_rust;

/// Returns all fix-code bench tasks.
pub fn tasks() -> Vec<BenchTask> {
    vec![
        fix_syntax_broken_rust::task(),
        fix_syntax_broken_python::task(),
        fix_logic_fizzbuzz::task(),
        fix_logic_sort::task(),
    ]
}
