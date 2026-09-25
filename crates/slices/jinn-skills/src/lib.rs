//! The skills slice — implementation services for agent-skill support.
//!
//! Owns YAML frontmatter parsing, directory scanning, prompt formatting, and
//! the loaded-skill label vocabulary. Portable skill values and crossing
//! contracts live in `jinn-skills-msg`. Everything here is UI-free and
//! kernel-free.

pub mod format;
pub mod frontmatter;
pub mod loaded_name;
pub mod scan;
pub mod skill;

pub use format::format_skills_for_prompt;
pub use jinn_skills_msg::{Skill, SkillFrontmatter, SkillSource};
pub use loaded_name::SKILL_CONTENT_PREFIX;
pub use loaded_name::SKILL_ICON;
pub use loaded_name::loaded_skill_summary_label;
pub use loaded_name::parse_loaded_skill_name;
pub use scan::scan_skills;
