//! edit-json-nested bench task — change a deeply nested timeout value in a JSON config.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;

const EXPECTED_CONFIG: &str = include_str!("edit_json_nested/expected/config.json");

pub fn task() -> BenchTask {
    BenchTask {
        name: "edit-json-nested",
        category: "edit",
        messages: vec![
            "In config.json, change the database connection timeout \
             (under environments > production > services > database > connection > timeout) \
             from 30 to 60. Do not modify any other timeout values.",
        ],
        fixture_dir: Some("src/tasks/edit/edit_json_nested/fixtures"),
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
    let checks = vec![checks::check_snapshot(dir, "config.json", EXPECTED_CONFIG)];
    VerificationReport::new("edit-json-nested", checks)
}
