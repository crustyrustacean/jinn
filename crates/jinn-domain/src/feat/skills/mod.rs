//! Agent skills - the kernel UI-bound surface.
//!
//! Portable values and crossing contracts live in `jinn-skills-msg`; parsing,
//! scanning, formatting, and loaded-name behavior live in `jinn-skills`. What
//! remains here is the kernel's UI-bound picker entry, preview cache, and reload
//! helper.

pub mod reload;
pub mod skill_entry;
pub mod skill_preview_cache;

pub use jinn_skills::format_skills_for_prompt;
pub use jinn_skills::frontmatter::strip_frontmatter;
pub use jinn_skills::loaded_skill_summary_label;
pub use jinn_skills::parse_loaded_skill_name;
pub use jinn_skills::scan_skills;
pub use skill_entry::SkillEntry;
pub use skill_preview_cache::SkillPreviewCache;
