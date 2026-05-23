//! edit-large-replace-small-file bench task — rewrite procedural Python into class-based.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;

const EXPECTED_MAIN_PY: &str = include_str!("edit_large_replace_small_file/expected/main.py");

pub fn task() -> BenchTask {
    BenchTask {
        name: "edit-large-replace-small-file",
        category: "edit",
        messages: vec![
            "Rewrite main.py as a proper class-based program. Create a `NumberProcessor` \
             class with methods: `load(data)` to set the numbers, `sum()` to return the sum, \
             `average()` to return the mean, `median()` to return the median, and `__str__` \
             for display. Keep the existing `main()` function but have it create a \
             `NumberProcessor` instance. Use the same data: [1, 5, 3, 9, 2, 7, 4, 8, 6].",
        ],
        fixture_dir: Some("src/tasks/edit/edit_large_replace_small_file/fixtures"),
        timeout: Duration::from_secs(300),
        persona: None,
        tools: BenchTools {
            builtins: vec!["read", "write"],
            custom: vec![],
        },
        verify,
    }
}

fn verify(dir: &Path) -> VerificationReport {
    let checks = vec![checks::check_snapshot(dir, "main.py", EXPECTED_MAIN_PY)];
    VerificationReport::new("edit-large-replace-small-file", checks)
}
