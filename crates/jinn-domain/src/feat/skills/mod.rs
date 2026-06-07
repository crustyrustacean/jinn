//! Agent skills - discovery, parsing, and data model.
//!
//! Scans `~/.agents/skills/*/SKILL.md` for skill definitions, parses their
//! YAML frontmatter, and provides the data model for skill metadata.

pub mod format;
pub mod frontmatter;
pub mod reload;
pub mod scan;
mod skill;
pub mod skill_entry;
pub mod skill_preview_cache;
pub mod skills_scan_actor;
pub mod loaded_name;

pub use scan::scan_skills;
pub use skill::Skill;
pub use skill_entry::SkillEntry;
pub use skill_preview_cache::SkillPreviewCache;
pub use skills_scan_actor::{ScanSkills, SkillsLoaded};
pub use loaded_name::parse_loaded_skill_name;
pub use loaded_name::{loaded_skill_summary_label, SKILL_ICON};

use std::path::PathBuf;

/// Returns the default agent skills directory: `~/.agents/skills/`.
pub fn skills_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agents")
        .join("skills")
}
