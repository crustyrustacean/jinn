//! Agent skills - discovery, parsing, and data model.
//!
//! Scans `~/.agents/skills/*/SKILL.md` for skill definitions, parses their
//! YAML frontmatter, and provides the data model for skill metadata.

pub mod format;
pub mod frontmatter;
pub mod loaded_name;
pub mod reload;
pub mod scan;
mod skill;
pub mod skill_entry;
pub mod skill_preview_cache;
pub mod skills_scan_actor;

pub use loaded_name::parse_loaded_skill_name;
pub use loaded_name::{SKILL_ICON, loaded_skill_summary_label};
pub use scan::scan_skills;
pub use skill::{Skill, SkillSource};
pub use skill_entry::SkillEntry;
pub use skill_preview_cache::SkillPreviewCache;
pub use skills_scan_actor::{ScanSkills, SkillsLoaded};
