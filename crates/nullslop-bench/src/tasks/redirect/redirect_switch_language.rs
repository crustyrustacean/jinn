//! redirect-switch-language bench task — add word counting, then rewrite in Rust.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;
use include_dir::Dir;


static FIXTURES: Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/tasks/redirect/redirect_switch_language/fixtures");

pub fn task() -> BenchTask {
    BenchTask {
        name: "redirect-switch-language",
        category: "redirect",
        messages: vec![
            "Add a feature to main.py that counts unique words and prints the count.",
            "Actually, rewrite the entire program in Rust instead. Save it to \
             src/main.rs. The Rust version should read input.txt and print word \
             count, character count, and unique word count.",
        ],
        fixture_dir: Some(&FIXTURES),
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
        checks::check_cargo_check(dir),
        checks::check_file_contains(dir, "src/main.rs", "input.txt"),
    ];
    VerificationReport::new("redirect-switch-language", checks)
}
