//! Skill preview cache — re-exported from the owning slice.
//!
//! The cache is the skill picker's own rendering cache, so it lives beside
//! the picker in `jinn-skills`. The kernel re-exports the name for callers
//! that still reach it through `FrontendCaches`.
pub use jinn_skills::skill_preview_cache::SkillPreviewCache;
