//! Shared fields parsed from a skill's YAML frontmatter.

/// Parsed frontmatter fields from a `SKILL.md` file.
#[derive(Debug, Clone)]
pub struct SkillFrontmatter {
    /// The skill name, which must match its parent directory.
    pub name: Option<String>,
    /// The skill description.
    pub description: Option<String>,
}
