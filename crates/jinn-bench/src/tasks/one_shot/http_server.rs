//! http-server bench task — write a minimal HTTP server (compile-check only).

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;

pub fn task() -> BenchTask {
    BenchTask {
        name: "http-server",
        category: "one_shot",
        messages: vec![
            "Write a minimal HTTP server in Rust (src/main.rs) that listens on \
             127.0.0.1:18091 and responds to GET / with \"ok\". Do NOT start the \
             server, just compile-check it with `cargo check`.",
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
        // Just needs to compile — we told it NOT to run.
        checks::check_cargo_check(dir),
    ];
    VerificationReport::new("http-server", checks)
}
