//! edit-surrounded-by-similar bench task - change one threshold among 10 nearly-identical blocks.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;
use include_dir::Dir;

const EXPECTED_MAIN_PY: &str = include_str!("edit_surrounded_by_similar/expected/main.py");

static FIXTURES: Dir<'_> = include_dir::include_dir!(
    "$CARGO_MANIFEST_DIR/src/tasks/edit/edit_surrounded_by_similar/fixtures"
);

pub fn task() -> BenchTask {
    BenchTask {
        name: "edit-surrounded-by-similar",
        category: "edit",
        messages: vec![
            "In main.py, change the temperature threshold for Tokyo from 28 to 32. \
             The Tokyo block is the 7th city check. Do not modify any other city's threshold.",
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
    let checks = vec![checks::check_snapshot(dir, "main.py", EXPECTED_MAIN_PY)];
    VerificationReport::new("edit-surrounded-by-similar", checks)
}
