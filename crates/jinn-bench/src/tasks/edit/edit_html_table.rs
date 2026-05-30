//! edit-html-table bench task - swap two specific rows in a 20-row HTML table.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;
use include_dir::Dir;

const EXPECTED_INDEX_HTML: &str = include_str!("edit_html_table/expected/index.html");

static FIXTURES: Dir<'_> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/tasks/edit/edit_html_table/fixtures");

pub fn task() -> BenchTask {
    BenchTask {
        name: "edit-html-table",
        category: "edit",
        messages: vec![
            "In index.html, swap the table row with id 5 with the table row with id 15. \
             Keep everything else exactly the same, including all formatting and indentation.",
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
    let checks = vec![checks::check_snapshot(
        dir,
        "index.html",
        EXPECTED_INDEX_HTML,
    )];
    VerificationReport::new("edit-html-table", checks)
}
