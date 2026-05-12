//! Agent skills — discovery, parsing, and data model.
//!
//! Scans `~/.agents/skills/*/SKILL.md` for skill definitions, parses their
//! YAML frontmatter, and provides the data model for skill metadata.

mod frontmatter;
mod scan;
mod skill;

pub use frontmatter::strip_frontmatter;
pub use scan::scan_skills;
pub use skill::Skill;
