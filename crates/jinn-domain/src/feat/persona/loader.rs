//! Persona file parser - reads markdown files with TOML frontmatter.
//!
//! Same format as prompt templates:
//!
//! ```markdown
//! +++
//! name = "coding-assistant"
//! description = "Expert coding assistant"
//! +++
//! You are an expert coding assistant...
//! ```

use std::path::Path;

use crate::feat::persona::Persona;
use error_stack::{Report, ResultExt as _};
use serde::Deserialize;
use wherror::Error;

/// Errors during persona file parsing.
#[derive(Debug, Error)]
#[error(debug)]
pub enum PersonaParseError {
    /// Filesystem I/O failure.
    Io,
    /// TOML frontmatter is missing or malformed.
    Frontmatter,
    /// TOML parsing error.
    Parse,
}

/// Frontmatter schema for persona files.
#[derive(Debug, Deserialize)]
struct Frontmatter {
    /// Unique persona name.
    name: String,
    /// Short description.
    #[serde(default)]
    description: String,
}

/// Parses a single persona file from disk.
///
/// # Errors
///
/// Returns an error if the file cannot be read or the frontmatter is malformed.
pub fn parse_persona_file(path: &Path) -> Result<Persona, Report<PersonaParseError>> {
    let content = std::fs::read_to_string(path)
        .change_context(PersonaParseError::Io)
        .attach(format!("failed to read {}", path.display()))?;
    parse_persona_content(&content, path)
}

/// Parses persona content string (testable without filesystem).
pub(crate) fn parse_persona_content(
    content: &str,
    path: &Path,
) -> Result<Persona, Report<PersonaParseError>> {
    let (frontmatter, body) = crate::common::frontmatter::parse_toml_frontmatter::<Frontmatter>(
        content,
    )
    .map_err(|report| {
        let ctx = match report.current_context() {
            crate::common::frontmatter::FrontmatterError::Parse => PersonaParseError::Parse,
            _ => PersonaParseError::Frontmatter,
        };
        report.change_context(ctx)
    })?;

    Ok(Persona {
        name: frontmatter.name,
        description: frontmatter.description,
        body,
        file_path: path.to_path_buf(),
    })
}

/// Scans a directory for persona files (`*.md`), returning all successfully parsed personas.
///
/// Files that fail to parse are logged as warnings and skipped.
/// Results are sorted by name for consistent ordering.
pub fn scan_personas_dir(dir: &Path) -> Vec<Persona> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };

    let mut personas = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md") {
            match parse_persona_file(&path) {
                Ok(persona) => personas.push(persona),
                Err(e) => {
                    tracing::warn!("failed to parse persona {}: {e:?}", path.display());
                }
            }
        }
    }

    personas.sort_by(|a, b| a.name.cmp(&b.name));
    personas
}

/// Scans both system and user persona directories, merging results.
///
/// System personas are loaded first. User personas with the same name
/// override system ones. Results are sorted by name.
pub fn scan_personas_merged(user_dir: &Path, system_dir: &Path) -> Vec<Persona> {
    let mut seen = std::collections::HashSet::new();
    let mut personas = Vec::new();

    // System personas first (lower priority).
    for persona in scan_personas_dir(system_dir) {
        seen.insert(persona.name.clone());
        personas.push(persona);
    }

    // User personas override system ones of the same name.
    for persona in scan_personas_dir(user_dir) {
        if seen.contains(&persona.name) {
            // Replace the system persona with the user version.
            if let Some(pos) = personas.iter().position(|p| p.name == persona.name)
                && let Some(slot) = personas.get_mut(pos)
            {
                *slot = persona;
            }
        } else {
            seen.insert(persona.name.clone());
            personas.push(persona);
        }
    }

    personas.sort_by(|a, b| a.name.cmp(&b.name));
    personas
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use std::path::PathBuf;

    fn test_path(name: &str) -> PathBuf {
        PathBuf::from(format!("/test/{name}.md"))
    }

    #[rstest::rstest]
    fn parse_persona_file_with_valid_frontmatter() {
        // Given a valid persona file content.
        let content = "+++\nname = \"coding-assistant\"\ndescription = \"Expert coder\"\n+++\n\nYou are an expert coding assistant.\n";

        // When parsing.
        let persona =
            parse_persona_content(content, &test_path("coding-assistant")).expect("parse");

        // Then fields are correctly extracted.
        assert_eq!(persona.name, "coding-assistant");
        assert_eq!(persona.description, "Expert coder");
        assert_eq!(persona.body, "You are an expert coding assistant.");
    }

    #[rstest::rstest]
    fn parse_persona_file_without_description() {
        // Given a persona file without description (uses default).
        let content = "+++\nname = \"minimal\"\n+++\n\nBody text here.";

        // When parsing.
        let persona = parse_persona_content(content, &test_path("minimal")).expect("parse");

        // Then description defaults to empty string.
        assert_eq!(persona.name, "minimal");
        assert_eq!(persona.description, "");
        assert_eq!(persona.body, "Body text here.");
    }

    #[rstest::rstest]
    fn parse_persona_file_fails_without_frontmatter() {
        // Given content without +++ delimiter.
        let content = "Just some text without frontmatter.";

        // When parsing.
        let result = parse_persona_content(content, &test_path("bad"));

        // Then it fails with Frontmatter error.
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().current_context(),
            PersonaParseError::Frontmatter
        ));
    }

    #[rstest::rstest]
    fn parse_persona_file_fails_without_closing_delimiter() {
        // Given content with opening +++ but no closing +++.
        let content = "+++\nname = \"test\"\nNo closing delimiter here.";

        // When parsing.
        let result = parse_persona_content(content, &test_path("bad"));

        // Then it fails with Frontmatter error.
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().current_context(),
            PersonaParseError::Frontmatter
        ));
    }

    #[rstest::rstest]
    fn parse_persona_file_fails_with_invalid_toml() {
        // Given content with invalid TOML in frontmatter.
        let content = "+++\nname = invalid toml\n+++\n\nBody.";

        // When parsing.
        let result = parse_persona_content(content, &test_path("bad"));

        // Then it fails with Parse error.
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().current_context(),
            PersonaParseError::Parse
        ));
    }

    #[rstest::rstest]
    fn parse_persona_file_preserves_multiline_body() {
        // Given a persona with multiline body.
        let content = "+++\nname = \"multi\"\n+++\n\nLine one.\nLine two.\nLine three.";

        // When parsing.
        let persona = parse_persona_content(content, &test_path("multi")).expect("parse");

        // Then all body lines are preserved.
        assert!(persona.body.contains("Line one."));
        assert!(persona.body.contains("Line two."));
        assert!(persona.body.contains("Line three."));
    }

    #[rstest::rstest]
    fn shipped_learning_tutor_persona_parses_with_its_grounding() {
        // Given the shipped learning-tutor persona bundled into the binary.
        let content = include_str!("../../../../../res/personas/learning-tutor.md");

        // When parsing it.
        let persona = parse_persona_content(content, &test_path("learning-tutor")).expect("parse");

        // Then it keeps its shipped name and an evidence-grounded description.
        assert_eq!(persona.name, "learning-tutor");
        assert!(persona.description.contains("intelligent-tutoring"));
    }

    #[rstest::rstest]
    fn scan_personas_dir_returns_sorted_personas() {
        // Given a directory with persona files.
        let dir = tempfile::TempDir::new().expect("temp dir");

        let beta = dir.path().join("beta.md");
        std::fs::write(
            &beta,
            "+++\nname = \"beta\"\ndescription = \"B\"\n+++\n\nBeta body.",
        )
        .expect("write");

        let alpha = dir.path().join("alpha.md");
        std::fs::write(
            &alpha,
            "+++\nname = \"alpha\"\ndescription = \"A\"\n+++\n\nAlpha body.",
        )
        .expect("write");

        // When scanning.
        let personas = scan_personas_dir(dir.path());

        // Then personas are sorted by name.
        assert_eq!(personas.len(), 2);
        assert_eq!(personas[0].name, "alpha");
        assert_eq!(personas[1].name, "beta");
    }

    #[rstest::rstest]
    fn scan_personas_dir_returns_empty_for_missing_dir() {
        // Given a nonexistent directory.
        let dir = PathBuf::from("/nonexistent/path");

        // When scanning.
        let personas = scan_personas_dir(&dir);

        // Then an empty vec is returned.
        assert!(personas.is_empty());
    }

    #[rstest::rstest]
    fn scan_personas_dir_skips_invalid_files() {
        // Given a directory with one valid and one invalid file.
        let dir = tempfile::TempDir::new().expect("temp dir");

        let valid = dir.path().join("valid.md");
        std::fs::write(
            &valid,
            "+++\nname = \"valid\"\ndescription = \"V\"\n+++\n\nValid body.",
        )
        .expect("write");

        let invalid = dir.path().join("invalid.md");
        std::fs::write(&invalid, "Not a valid persona file.").expect("write");

        // When scanning.
        let personas = scan_personas_dir(dir.path());

        // Then only the valid persona is returned.
        assert_eq!(personas.len(), 1);
        assert_eq!(personas[0].name, "valid");
    }

    #[rstest::rstest]
    fn scan_personas_dir_ignores_non_md_files() {
        // Given a directory with a .txt file.
        let dir = tempfile::TempDir::new().expect("temp dir");

        let txt = dir.path().join("notes.txt");
        std::fs::write(
            &txt,
            "+++\nname = \"hidden\"\ndescription = \"H\"\n+++\n\nBody.",
        )
        .expect("write");

        // When scanning.
        let personas = scan_personas_dir(dir.path());

        // Then no personas are found.
        assert!(personas.is_empty());
    }

    #[rstest::rstest]
    fn scan_personas_merged_user_overrides_system() {
        // Given system and user dirs with the same persona name.
        let system_dir = tempfile::TempDir::new().expect("temp dir");
        let user_dir = tempfile::TempDir::new().expect("temp dir");

        std::fs::write(
            system_dir.path().join("shared.md"),
            "+++\nname = \"shared\"\ndescription = \"System version\"\n+++\n\nSystem body.",
        )
        .expect("write");
        std::fs::write(
            user_dir.path().join("shared.md"),
            "+++\nname = \"shared\"\ndescription = \"User version\"\n+++\n\nUser body.",
        )
        .expect("write");

        // When scanning merged.
        let personas = scan_personas_merged(user_dir.path(), system_dir.path());

        // Then the user version overrides the system version.
        assert_eq!(personas.len(), 1);
        assert_eq!(personas[0].description, "User version");
        assert_eq!(personas[0].body, "User body.");
    }

    #[rstest::rstest]
    fn scan_personas_merged_returns_both_when_different_names() {
        // Given system dir with "alpha" and user dir with "beta".
        let system_dir = tempfile::TempDir::new().expect("temp dir");
        let user_dir = tempfile::TempDir::new().expect("temp dir");

        std::fs::write(
            system_dir.path().join("alpha.md"),
            "+++\nname = \"alpha\"\ndescription = \"A\"\n+++\n\nAlpha body.",
        )
        .expect("write");
        std::fs::write(
            user_dir.path().join("beta.md"),
            "+++\nname = \"beta\"\ndescription = \"B\"\n+++\n\nBeta body.",
        )
        .expect("write");

        // When scanning merged.
        let personas = scan_personas_merged(user_dir.path(), system_dir.path());

        // Then both personas are returned (sorted by name).
        assert_eq!(personas.len(), 2);
        assert_eq!(personas[0].name, "alpha");
        assert_eq!(personas[1].name, "beta");
    }

    #[rstest::rstest]
    fn scan_personas_merged_returns_non_empty() {
        // Given both dirs with one persona each (different names).
        let system_dir = tempfile::TempDir::new().expect("temp dir");
        let user_dir = tempfile::TempDir::new().expect("temp dir");

        std::fs::write(
            system_dir.path().join("sys.md"),
            "+++\nname = \"sys\"\ndescription = \"S\"\n+++\n\nSys body.",
        )
        .expect("write");

        // When scanning merged.
        let personas = scan_personas_merged(user_dir.path(), system_dir.path());

        // Then at least the system persona is returned.
        assert!(!personas.is_empty());
        assert_eq!(personas[0].name, "sys");
    }
}
