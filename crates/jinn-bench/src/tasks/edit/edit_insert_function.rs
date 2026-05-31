//! edit-insert-function bench task - insert a new function between two existing ones.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;
use include_dir::Dir;

const EXPECTED_MAIN_RS: &str = include_str!("edit_insert_function/expected/src/main.rs");

static FIXTURES: Dir<'_> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/tasks/edit/edit_insert_function/fixtures");

pub fn task() -> BenchTask {
    BenchTask {
        name: "edit-insert-function",
        category: "edit",
        messages: vec![
            "Add a `divide` function to src/main.rs. It should take two i32 parameters \
             (a and b), return f64, and return 0.0 if b is 0. Insert it between the \
             `subtract` and `multiply` functions. Also add a call to it in main(). \
             Keep the existing formatting style.",
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
    let checks = vec![checks::check_snapshot(dir, "src/main.rs", EXPECTED_MAIN_RS)];
    VerificationReport::new("edit-insert-function", checks)
}
