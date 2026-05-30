//! fix-logic-sort bench task — fix bubble sort index out of bounds.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;
use include_dir::Dir;

static FIXTURES: Dir<'_> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/tasks/fix_code/fix_logic_sort/fixtures");

pub fn task() -> BenchTask {
    BenchTask {
        name: "fix-logic-sort",
        category: "fix_code",
        messages: vec![
            "The bubble sort in src/main.rs has a bug that causes an index out of \
             bounds panic. Find and fix it, then run the program to confirm it sorts \
             correctly.",
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
        checks::check_cargo_run(dir),
        checks::check_cargo_run_contains(dir, "11"),
        checks::check_cargo_run_contains(dir, "90"),
        checks::check_cargo_run_contains(dir, "Sorted"),
    ];
    VerificationReport::new("fix-logic-sort", checks)
}
