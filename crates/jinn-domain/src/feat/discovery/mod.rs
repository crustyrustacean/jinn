//! Project-local resource discovery - bounded cwd→ancestor walk.
//!
//! Resolves the set of project directories that contribute skills, prompts, and
//! AGENTS.md/CLAUDE.md context files for a given session cwd. The walk is
//! **bounded**: it stops at the first VCS-marker root (inclusive) or at `$HOME`
//! (exclusive), whichever comes first.
//!
//! This module is pure path resolution. It performs cheap `std::fs::metadata`
//! checks for VCS markers but does no file content reading. The scan actors read
//! the resolved directories; this module only decides _which_ directories count.
//!
//! See [`.plans/project-locals/plan.md`](../../../.plans/project-locals/plan.md)
//! decision D4 for the stopping-rule rationale.

pub mod vcs;
pub mod walk;

pub use vcs::is_vcs_root;
pub use walk::{project_context_files, project_dirs, project_prompts_dirs, project_skills_dirs};

/// Relative location of project skills under a project root.
pub const SKILLS_SUBDIR: &str = ".agents/skills";

/// Relative location of project prompts under a project root.
pub const PROMPTS_SUBDIR: &str = ".agents/prompts";

/// Candidate project context filenames, checked in order.
///
/// Mirrors `crate::feat::context::env_context::CONTEXT_FILE_CANDIDATES`. Duplicated
/// here so the discovery module is self-contained (no upward dep on env_context).
pub const CONTEXT_FILE_CANDIDATES: &[&str] = &["AGENTS.md", "AGENTS.MD", "CLAUDE.md", "CLAUDE.MD"];
