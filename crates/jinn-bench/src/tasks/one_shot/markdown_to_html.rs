//! markdown-to-html bench task - convert markdown to HTML with file I/O.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;

pub fn task() -> BenchTask {
    BenchTask {
        name: "markdown-to-html",
        category: "one_shot",
        messages: vec![
            "Write a Rust program (src/main.rs) that converts a markdown file to HTML. \
             Create a file called input.md with some markdown content (headings, bold, \
             italic, a list, and a code block). The program should read input.md and \
             write output.html. Run the program.",
        ],
        fixture_dir: None,
        timeout: Duration::from_secs(300),
        persona: None,
        tools: BenchTools {
            builtins: vec!["bash", "read", "write"],
            custom: vec![],
        },
        verify,
    }
}

fn verify(dir: &Path) -> VerificationReport {
    let checks = vec![
        checks::check_file_exists(dir, "src/main.rs"),
        checks::check_file_exists(dir, "input.md"),
        checks::check_file_exists(dir, "output.html"),
        checks::check_cargo_check(dir),
    ];
    VerificationReport::new("markdown-to-html", checks)
}
