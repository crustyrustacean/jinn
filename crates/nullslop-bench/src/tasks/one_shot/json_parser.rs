//! json-parser bench task — parse a JSON file and print name + age.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;

pub fn task() -> BenchTask {
    BenchTask {
        name: "json-parser",
        messages: vec![
            "Write a Rust program that parses a JSON file containing an array of \
             objects with \"name\" (string) and \"age\" (number) fields, then prints \
             each person's name and age. Create a test file at input.json with at \
             least 3 people, save the program to src/main.rs, and run it.",
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
        checks::check_file_exists(dir, "input.json"),
        checks::check_cargo_check(dir),
    ];
    VerificationReport::new("json-parser", checks)
}
