//! Workflow scanning — the thin delegation to the workflows slice's scan.
//!
//! The scan kernel lives in [`jinn_workflows::scan_workflows`]; this module
//! exists so the worker's resource list reads like the other three
//! (`scans::scan_skills_merged`, `scans::load_prompts`,
//! `scans::read_context_files`, `scans::scan_workflows`). It adds no logic
//! of its own — a third copy of scan logic is exactly what this must not
//! become.

use jinn_workflows_msg::Workflow;
use marked_path::{Absolute, MarkedPathBuf};

/// Scans the workflows directory for admitted workflows.
///
/// Delegates to [`jinn_workflows::scan_workflows`]: `*/workflow.md` in
/// direct subdirectories, `+++` TOML frontmatter required, non-blank
/// `description` required, directory name as the name fallback. A
/// nonexistent dir yields an empty list.
#[must_use]
pub fn scan_workflows(dir: &MarkedPathBuf<Absolute>) -> Vec<Workflow> {
    jinn_workflows::scan_workflows(dir)
}
