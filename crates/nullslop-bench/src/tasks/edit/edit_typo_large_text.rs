//! edit-typo-large-text bench task — fix a single typo in a large prose file.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;

const EXPECTED_PROSE: &str = include_str!("edit_typo_large_text/expected/prose.txt");

pub fn task() -> BenchTask {
    BenchTask {
        name: "edit-typo-large-text",
        category: "edit",
        messages: vec![
            "Fix the typo in prose.txt. The word 'accomodate' should be spelled 'accommodate'.",
        ],
        fixture_dir: Some("src/tasks/edit/edit_typo_large_text/fixtures"),
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
    let checks = vec![checks::check_snapshot(dir, "prose.txt", EXPECTED_PROSE)];
    VerificationReport::new("edit-typo-large-text", checks)
}
