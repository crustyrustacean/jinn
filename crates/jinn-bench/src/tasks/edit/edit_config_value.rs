//! edit-config-value bench task - change a single port number in a YAML config.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, VerificationReport};
use crate::tasks::checks;
use include_dir::Dir;

const EXPECTED_CONFIG: &str = include_str!("edit_config_value/expected/config.yaml");

static FIXTURES: Dir<'_> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/tasks/edit/edit_config_value/fixtures");

pub fn task() -> BenchTask {
    BenchTask {
        name: "edit-config-value",
        category: "edit",
        messages: vec!["Change the server port from 8080 to 9090 in config.yaml."],
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
    let checks = vec![checks::check_snapshot(dir, "config.yaml", EXPECTED_CONFIG)];
    VerificationReport::new("edit-config-value", checks)
}
