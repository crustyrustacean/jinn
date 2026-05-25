//! fix-syntax-broken-python bench task — fix syntax errors in Python code.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;
use include_dir::Dir;

static FIXTURES: Dir<'_> = include_dir::include_dir!(
    "$CARGO_MANIFEST_DIR/src/tasks/fix_code/fix_syntax_broken_python/fixtures"
);

pub fn task() -> BenchTask {
    BenchTask {
        name: "fix-syntax-broken-python",
        category: "fix_code",
        messages: vec![
            "There are syntax errors in main.py. Find and fix them all, then run the \
             program with `python main.py` to confirm it prints the fibonacci sequence \
             from fib(0) to fib(9).",
        ],
        fixture_dir: Some(&FIXTURES),
        timeout: Duration::from_secs(300),
        persona: None,
        tools: BenchTools {
            builtins: vec!["bash", "read", "write"],
            custom: vec![],
        },
        verify,
    }
}

fn verify(dir: &Path) -> VerificationReport {
    let checks = vec![
        checks::check_python_run(dir, "main.py"),
        checks::check_python_run_contains(dir, "main.py", "0"),
        checks::check_python_run_contains(dir, "main.py", "34"),
    ];
    VerificationReport::new("fix-syntax-broken-python", checks)
}
