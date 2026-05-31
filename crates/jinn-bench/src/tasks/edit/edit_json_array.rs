//! edit-json-array bench task - remove one specific object from a JSON array of 20 similar ones.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;
use include_dir::Dir;

const EXPECTED_DATA: &str = include_str!("edit_json_array/expected/data.json");

static FIXTURES: Dir<'_> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/tasks/edit/edit_json_array/fixtures");

pub fn task() -> BenchTask {
    BenchTask {
        name: "edit-json-array",
        category: "edit",
        messages: vec![
            "Remove the user with id 17 from data.json. Keep everything else exactly the same.",
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
    let checks = vec![checks::check_snapshot(dir, "data.json", EXPECTED_DATA)];
    VerificationReport::new("edit-json-array", checks)
}
