//! The skills slice — implementation services for agent-skill support.
//!
//! Owns YAML frontmatter parsing, directory scanning, prompt formatting, and
//! the loaded-skill label vocabulary. Portable skill values and crossing
//! contracts live in `jinn-skills-msg`. Everything here is UI-free and
//! kernel-free.

pub mod format;
pub mod frontmatter;
pub mod scan;
pub mod skill;
pub mod skill_preview;

pub use format::format_skills_for_prompt;
pub use jinn_skills_msg::SKILL_CONTENT_PREFIX;
pub use jinn_skills_msg::SKILL_ICON;
pub use jinn_skills_msg::loaded_skill_summary_label;
pub use jinn_skills_msg::parse_loaded_skill_name;
pub use jinn_skills_msg::{Skill, SkillFrontmatter, SkillSource};
pub use scan::scan_skills;
pub use skill_preview::render_skill_preview;
