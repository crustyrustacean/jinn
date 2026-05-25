//! edit-duplicate-sections bench task — change one field type in one of 5 similar structs.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;
use include_dir::Dir;

const EXPECTED_MAIN_RS: &str = include_str!("edit_duplicate_sections/expected/src/main.rs");

static FIXTURES: Dir<'_> = include_dir::include_dir!(
    "$CARGO_MANIFEST_DIR/src/tasks/edit/edit_duplicate_sections/fixtures"
);

pub fn task() -> BenchTask {
    BenchTask {
        name: "edit-duplicate-sections",
        category: "edit",
        messages: vec![
            "In src/main.rs, change the `components` field type in the `CmykColor` struct \
             from `Vec<f32>` to `Vec<u8>`. Do not change anything else.",
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
    VerificationReport::new("edit-duplicate-sections", checks)
}
