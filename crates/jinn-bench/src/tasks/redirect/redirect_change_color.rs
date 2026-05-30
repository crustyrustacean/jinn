//! redirect-change-color bench task - change background color, then redirect to different color.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, CheckResult, VerificationReport};
use include_dir::Dir;

static FIXTURES: Dir<'_> = include_dir::include_dir!(
    "$CARGO_MANIFEST_DIR/src/tasks/redirect/redirect_change_color/fixtures"
);

pub fn task() -> BenchTask {
    BenchTask {
        name: "redirect-change-color",
        category: "redirect",
        messages: vec![
            "Change the background color of index.html from blue to red.",
            "Actually, I changed my mind - make the background dark gray (#333333) \
             instead and change the heading color to orange.",
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
    let content = std::fs::read_to_string(dir.join("index.html")).unwrap_or_default();

    // Final state: background should be dark gray (#333333), heading should be orange.
    let bg_ok = content.contains("#333")
        || content.contains("#333333")
        || content.contains("darkgray")
        || content.contains("dark gray");
    let heading_ok =
        content.contains("orange") || content.contains("#ff") || content.contains("#FF");

    let checks = vec![
        if bg_ok {
            CheckResult::pass("background_dark_gray")
        } else {
            CheckResult::fail(
                "background_dark_gray",
                "expected background color #333/#333333/darkgray",
            )
        },
        if heading_ok {
            CheckResult::pass("heading_orange")
        } else {
            CheckResult::fail("heading_orange", "expected heading color orange")
        },
    ];
    VerificationReport::new("redirect-change-color", checks)
}
