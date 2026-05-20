//! Agent skills — discovery, parsing, and data model.
//!
//! Scans `~/.agents/skills/*/SKILL.md` for skill definitions, parses their
//! YAML frontmatter, and provides the data model for skill metadata.

pub mod format;
pub mod frontmatter;
pub mod scan;
mod skill;
pub mod skills_scan_actor;

pub use scan::scan_skills;
pub use skill::Skill;
pub use skills_scan_actor::{ScanSkills, SkillsLoaded};

use std::path::PathBuf;

/// Returns the default agent skills directory: `~/.agents/skills/`.
pub fn skills_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agents")
        .join("skills")
}
