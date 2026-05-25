//! fix-logic-fizzbuzz bench task — fix FizzBuzz logic bug.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;
use include_dir::Dir;

static FIXTURES: Dir<'_> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/tasks/fix_code/fix_logic_fizzbuzz/fixtures");

pub fn task() -> BenchTask {
    BenchTask {
        name: "fix-logic-fizzbuzz",
        category: "fix_code",
        messages: vec![
            "The FizzBuzz program in src/main.rs has a logic bug — it never prints \
             \"FizzBuzz\". Find and fix the bug, then run the program. The correct \
             output for 15 should be \"FizzBuzz\", not \"Fizz\".",
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
    let checks = vec![checks::check_fizzbuzz_output(dir)];
    VerificationReport::new("fix-logic-fizzbuzz", checks)
}
