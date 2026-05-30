//! Noop bench task for testing — completes instantly, no fixtures, no subprocesses.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};

pub fn task() -> BenchTask {
    BenchTask {
        name: "test-noop",
        category: "test",
        messages: vec!["noop"],
        fixture_dir: None,
        timeout: Duration::from_secs(5),
        persona: None,
        tools: BenchTools {
            builtins: vec![],
            custom: vec![],
        },
        verify,
    }
}

fn verify(_dir: &Path) -> VerificationReport {
    VerificationReport::new("test-noop", vec![])
}
