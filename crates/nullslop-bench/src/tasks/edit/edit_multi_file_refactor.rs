//! edit-multi-file-refactor bench task — rename a function across lib.rs and main.rs.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;
use include_dir::Dir;

const EXPECTED_LIB_RS: &str = include_str!("edit_multi_file_refactor/expected/src/lib.rs");
const EXPECTED_MAIN_RS: &str = include_str!("edit_multi_file_refactor/expected/src/main.rs");


static FIXTURES: Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/tasks/edit/edit_multi_file_refactor/fixtures");

pub fn task() -> BenchTask {
    BenchTask {
        name: "edit-multi-file-refactor",
        category: "edit",
        messages: vec![
            "Rename the `calculate_discount` function to `apply_discount`. Update both \
             src/lib.rs (where it's defined) and src/main.rs (where it's called). \
             Do not change anything else.",
        ],
        fixture_dir: Some(&FIXTURES),
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
    let checks = vec![
        checks::check_snapshot(dir, "src/lib.rs", EXPECTED_LIB_RS),
        checks::check_snapshot(dir, "src/main.rs", EXPECTED_MAIN_RS),
    ];
    VerificationReport::new("edit-multi-file-refactor", checks)
}
