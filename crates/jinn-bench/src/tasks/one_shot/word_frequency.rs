//! word-frequency bench task - count word frequencies and print top 5.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;

pub fn task() -> BenchTask {
    BenchTask {
        name: "word-frequency",
        category: "one_shot",
        messages: vec![
            "Create a text file called input.txt with a few sentences of your choice. \
             Then write a Rust program (src/main.rs) that reads input.txt, counts word \
             frequencies, and prints the top 5 most common words with their counts. \
             Run the program.",
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
        checks::check_file_exists(dir, "input.txt"),
        checks::check_cargo_check(dir),
    ];
    VerificationReport::new("word-frequency", checks)
}
