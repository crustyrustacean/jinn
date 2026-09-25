//! Agent skills - the kernel UI-bound surface.
//!
//! Portable values and crossing contracts live in `jinn-skills-msg`; parsing,
//! scanning, formatting, and loaded-name behavior live in `jinn-skills`. What
//! remains here is the kernel's UI-bound picker entry, preview cache, and reload
//! helper.
//!
//! This module deliberately re-exports nothing from `jinn-skills`: every real
//! consumer imports that crate directly, and a re-export would make the kernel a
//! production dependent of the slice - which would, in turn, forbid the slice
//! from depending on the kernel and so freeze the skill picker in this crate
//! forever.

pub mod reload;
pub mod skill_entry;
pub mod skill_preview_cache;

pub use skill_entry::SkillEntry;
pub use skill_preview_cache::SkillPreviewCache;
