//! Skill picker entry type — re-exported from the owning slice.
//!
//! The entry type, its row renderer, and its preview cache key now live in
//! `jinn-skills-msg`, alongside the picker's state cell payload. The skills
//! slice owns the picker outright; the kernel re-exports the names so the
//! migration lands without a flag day.

pub use jinn_skills::render_skill_preview;
pub use jinn_skills_msg::SkillEntry;
pub use jinn_skills_msg::body_hash_key;
pub use jinn_skills_msg::skill_row;
