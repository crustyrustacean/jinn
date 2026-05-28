//! Skill scanning — discovers SKILL.md files in a directory tree.

use std::path::Path;

use super::Skill;
use super::frontmatter::parse_frontmatter;

/// Scans a directory for agent skills.
///
/// Looks for `*/SKILL.md` files in direct subdirectories of `dir`.
/// Parses YAML frontmatter for name and description.
/// Skips entries without valid frontmatter or description.
/// Returns an empty vector if the directory does not exist.
pub fn scan_skills(dir: &Path) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut skills = Vec::new();

    for entry in entries.flatten() {
        let skill_md = entry.path().join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&skill_md) else {
            continue;
        };

        let Some(frontmatter) = parse_frontmatter(&content) else {
            continue;
        };

        let Some(description) = frontmatter.description else {
            continue;
        };

        if description.trim().is_empty() {
            continue;
        }

        let name = frontmatter
            .name
            .unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());

        skills.push(Skill {
            name,
            description,
            file_path: skill_md,
            base_dir: entry.path(),
        });
    }

    skills
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use std::fs;

    #[rstest::rstest]
    fn scan_skills_finds_valid_skill() {
        // Given a temp directory with a valid skill.
        let dir = tempfile::tempdir().expect("create temp dir");
        let skill_dir = dir.path().join("my-skill");
        fs::create_dir_all(&skill_dir).expect("create skill dir");
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: A test skill\n---\n\n# Content",
        )
        .expect("write SKILL.md");

        // When scanning for skills.
        let skills = scan_skills(dir.path());

        // Then one skill is found.
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my-skill");
        assert_eq!(skills[0].description, "A test skill");
        assert_eq!(skills[0].file_path, skill_dir.join("SKILL.md"));
        assert_eq!(skills[0].base_dir, skill_dir);
    }

    #[rstest::rstest]
    fn scan_skills_uses_dir_name_when_no_frontmatter_name() {
        // Given a skill without name in frontmatter.
        let dir = tempfile::tempdir().expect("create temp dir");
        let skill_dir = dir.path().join("fallback-name");
        fs::create_dir_all(&skill_dir).expect("create skill dir");
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\ndescription: Has desc but no name\n---\n\n# Content",
        )
        .expect("write SKILL.md");

        // When scanning for skills.
        let skills = scan_skills(dir.path());

        // Then the directory name is used.
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "fallback-name");
    }

    #[rstest::rstest]
    fn scan_skills_skips_empty_description() {
        // Given a skill with an empty description.
        let dir = tempfile::tempdir().expect("create temp dir");
        let skill_dir = dir.path().join("empty-desc");
        fs::create_dir_all(&skill_dir).expect("create skill dir");
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: empty-desc\ndescription: \n---\n\n# Content",
        )
        .expect("write SKILL.md");

        // When scanning for skills.
        let skills = scan_skills(dir.path());

        // Then no skills are found.
        assert!(skills.is_empty());
    }

    #[rstest::rstest]
    fn scan_skills_skips_missing_description() {
        // Given a skill without description.
        let dir = tempfile::tempdir().expect("create temp dir");
        let skill_dir = dir.path().join("no-desc");
        fs::create_dir_all(&skill_dir).expect("create skill dir");
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: no-desc\n---\n\n# Content",
        )
        .expect("write SKILL.md");

        // When scanning for skills.
        let skills = scan_skills(dir.path());

        // Then no skills are found.
        assert!(skills.is_empty());
    }

    #[rstest::rstest]
    fn scan_skills_skips_dirs_without_skill_md() {
        // Given a directory with a subdirectory but no SKILL.md.
        let dir = tempfile::tempdir().expect("create temp dir");
        let sub = dir.path().join("no-skill-here");
        fs::create_dir_all(&sub).expect("create sub dir");

        // When scanning for skills.
        let skills = scan_skills(dir.path());

        // Then no skills are found.
        assert!(skills.is_empty());
    }

    #[rstest::rstest]
    fn scan_skills_returns_empty_for_nonexistent_dir() {
        // Given a nonexistent directory.
        let dir = Path::new("/nonexistent/path");

        // When scanning for skills.
        let skills = scan_skills(dir);

        // Then no skills are found.
        assert!(skills.is_empty());
    }

    #[rstest::rstest]
    fn scan_skills_skips_invalid_frontmatter() {
        // Given a skill dir with SKILL.md but no frontmatter.
        let dir = tempfile::tempdir().expect("create temp dir");
        let skill_dir = dir.path().join("no-frontmatter");
        fs::create_dir_all(&skill_dir).expect("create skill dir");
        fs::write(
            skill_dir.join("SKILL.md"),
            "# Just markdown\n\nNo frontmatter.",
        )
        .expect("write SKILL.md");

        // When scanning for skills.
        let skills = scan_skills(dir.path());

        // Then no skills are found.
        assert!(skills.is_empty());
    }

    #[rstest::rstest]
    fn scan_skills_finds_multiple_skills() {
        // Given a directory with multiple skills.
        let dir = tempfile::tempdir().expect("create temp dir");

        for name in ["skill-a", "skill-b", "skill-c"] {
            let skill_dir = dir.path().join(name);
            fs::create_dir_all(&skill_dir).expect("create skill dir");
            fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: Skill {name}\n---\n\n# {name}"),
            )
            .expect("write SKILL.md");
        }

        // When scanning for skills.
        let skills = scan_skills(dir.path());

        // Then three skills are found.
        assert_eq!(skills.len(), 3);
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"skill-a"));
        assert!(names.contains(&"skill-b"));
        assert!(names.contains(&"skill-c"));
    }
}
