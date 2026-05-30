//! redirect-refactor-function bench task — add volume, then switch to paint estimation.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, CheckResult, VerificationReport};
use include_dir::Dir;

static FIXTURES: Dir<'_> = include_dir::include_dir!(
    "$CARGO_MANIFEST_DIR/src/tasks/redirect/redirect_refactor_function/fixtures"
);

pub fn task() -> BenchTask {
    BenchTask {
        name: "redirect-refactor-function",
        category: "redirect",
        messages: vec![
            "Add a function to main.py called calculate_volume that takes length, \
             width, and height, and returns the volume. Update main() to also print \
             the volume for each room, assuming a ceiling height of 2.5m.",
            "Wait, I don't need volume. Remove calculate_volume and instead add a \
             function called calculate_paint_needed that estimates paint in liters \
             (area * 0.1 liters per square meter for walls). Print that instead.",
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
    let content = std::fs::read_to_string(dir.join("main.py")).unwrap_or_default();

    // Should have calculate_paint_needed (or similar paint function), NOT calculate_volume.
    let has_paint = content.contains("paint")
        || content.contains("Paint")
        || content.contains("liter")
        || content.contains("litre");
    let no_volume = !content.contains("volume") && !content.contains("Volume");

    let checks = vec![
        if has_paint {
            CheckResult::pass("has_paint_reference")
        } else {
            CheckResult::fail(
                "has_paint_reference",
                "expected paint/liter reference in main.py",
            )
        },
        if no_volume {
            CheckResult::pass("no_volume_reference")
        } else {
            CheckResult::fail(
                "no_volume_reference",
                "expected no volume/Volume reference in main.py",
            )
        },
    ];
    VerificationReport::new("redirect-refactor-function", checks)
}
