//! Portable agent-skill values shared by skill messages and implementations.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where a [`Skill`] was discovered from.
///
/// Used to badge entries in the skill picker (global vs project-scoped)
/// and to resolve provenance when a project skill overrides a global one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
pub enum SkillSource {
    /// Discovered from the user-global skills dir (`~/.agents/skills`).
    #[default]
    Global,
    /// Discovered from a project-local `.agents/skills` directory.
    Project {
        /// The walked directory containing `.agents/`, rather than the
        /// `.agents/skills` subdirectory itself.
        dir: PathBuf,
    },
}

/// A discovered agent skill.
///
/// Parsed from `SKILL.md` files in `~/.agents/skills/<name>/`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Skill {
    /// The skill name from frontmatter, which must match its parent directory.
    pub name: String,
    /// Human-readable description of what the skill does.
    pub description: String,
    /// The markdown body after stripping YAML frontmatter.
    ///
    /// This is not serialized because skills are loaded freshly from disk.
    #[serde(skip)]
    pub body: String,
    /// Absolute path to the `SKILL.md` file.
    pub file_path: PathBuf,
    /// Absolute path to the directory containing `SKILL.md`.
    pub base_dir: PathBuf,
    /// Where this skill was discovered from.
    #[serde(default)]
    pub source: SkillSource,
}
