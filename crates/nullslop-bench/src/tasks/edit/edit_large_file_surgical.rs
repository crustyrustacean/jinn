//! edit-large-file-surgical bench task — change exactly one specific 1024 to 4096 in a ~360-line file.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;
use include_dir::Dir;

const EXPECTED_MAIN_RS: &str = include_str!("edit_large_file_surgical/expected/main.rs");


static FIXTURES: Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/tasks/edit/edit_large_file_surgical/fixtures");

pub fn task() -> BenchTask {
    BenchTask {
        name: "edit-large-file-surgical",
        category: "edit",
        messages: vec![
            "In main.rs, change the initial read buffer size in the `read_file` function \
             from 1024 to 4096. This is the line `let mut buffer = vec![0u8; 1024];` \
             near the middle of the file. Only change that specific occurrence — there \
             are other uses of 1024 in the file that should not be modified.",
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
    let checks = vec![checks::check_snapshot(dir, "main.rs", EXPECTED_MAIN_RS)];
    VerificationReport::new("edit-large-file-surgical", checks)
}
