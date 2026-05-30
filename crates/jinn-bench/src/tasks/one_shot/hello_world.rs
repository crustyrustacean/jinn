//! hello-world bench task - write and run a Rust hello world program.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;
use include_dir::Dir;

static FIXTURES: Dir<'_> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/tasks/one_shot/hello_world/fixtures");

pub fn task() -> BenchTask {
    BenchTask {
        name: "hello-world",
        category: "one_shot",
        messages: vec!["Write a hello world program in Rust. Save it to src/main.rs and run it."],
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
    ];
    VerificationReport::new("hello-world", checks)
}
