//! edit-rename-all bench task — rename variable `counter` to `item_count` everywhere.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;

const EXPECTED_MAIN_PY: &str = include_str!("edit_rename_all/expected/main.py");

pub fn task() -> BenchTask {
    BenchTask {
        name: "edit-rename-all",
        category: "edit",
        messages: vec![
            "Rename the variable `counter` to `item_count` everywhere it appears in \
             main.py. Do not change anything else.",
        ],
        fixture_dir: Some("src/tasks/edit/edit_rename_all/fixtures"),
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
    VerificationReport::new("edit-rename-all", checks)
}
