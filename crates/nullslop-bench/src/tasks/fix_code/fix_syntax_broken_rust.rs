//! fix-syntax-broken-rust bench task — fix a syntax error in Rust code.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;

pub fn task() -> BenchTask {
    BenchTask {
        name: "fix-syntax-broken-rust",
        messages: vec![
            "There is a syntax error in src/main.rs. Find and fix it, then run the \
             program with `cargo run` to confirm it prints the correct sum (15).",
        ],
        fixture_dir: Some("src/tasks/fix_code/fix_syntax_broken_rust/fixtures"),
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
    let checks = vec![checks::check_cargo_check(dir)];
    VerificationReport::new("fix-syntax-broken-rust", checks)
}
