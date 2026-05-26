// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Judge file parser — reads markdown files with TOML frontmatter.
//!
//! Same format as personas and prompt templates:
//!
//! ```markdown
//! +++
//! name = "accuracy"
//! description = "Verifies code accuracy against requirements"
//! model = "anthropic/claude-sonnet"  # optional
//! +++
//!
//! The agent has finished its task. Please confirm...
//! ```

use std::path::Path;

use error_stack::{Report, ResultExt as _};
use serde::Deserialize;
use wherror::Error;

use super::Judge;

/// Errors during judge file parsing.
#[derive(Debug, Error)]
#[error(debug)]
pub enum JudgeParseError {
    /// Filesystem I/O failure.
    Io,
    /// TOML frontmatter is missing or malformed.
    Frontmatter,
    /// TOML parsing error.
    Parse,
}

/// Frontmatter schema for judge files.
#[derive(Debug, Deserialize)]
struct Frontmatter {
    /// Unique judge name.
    name: String,
    /// Short description.
    #[serde(default)]
    description: String,
    /// Optional model override for the judge session.
    #[serde(default)]
    model: Option<String>,
    /// Whether this judge auto-resets history before each evaluation.
    #[serde(default)]
    auto_reset: bool,
}

/// Parses a single judge file from disk.
///
/// # Errors
///
/// Returns an error if the file cannot be read or the frontmatter is malformed.
pub fn parse_judge_file(path: &Path) -> Result<Judge, Report<JudgeParseError>> {
    let content = std::fs::read_to_string(path)
        .change_context(JudgeParseError::Io)
        .attach(format!("failed to read {}", path.display()))?;
    parse_judge_content(&content, path)
}

/// Parses judge content string (testable without filesystem).
pub(crate) fn parse_judge_content(
    content: &str,
    path: &Path,
) -> Result<Judge, Report<JudgeParseError>> {
    let (frontmatter, body) = crate::common::frontmatter::parse_toml_frontmatter::<Frontmatter>(
        content,
    )
    .map_err(|report| {
        let ctx = match report.current_context() {
            crate::common::frontmatter::FrontmatterError::Parse => JudgeParseError::Parse,
            _ => JudgeParseError::Frontmatter,
        };
        report.change_context(ctx)
    })?;

    Ok(Judge {
        name: frontmatter.name,
        description: frontmatter.description,
        body,
        model: frontmatter.model,
        auto_reset: frontmatter.auto_reset,
        file_path: path.to_path_buf(),
    })
}

/// Scans a directory for judge files (`*.md`), returning all successfully parsed judges.
///
/// Files that fail to parse are logged as warnings and skipped.
/// Results are sorted by name for consistent ordering.
pub fn scan_judges_dir(dir: &Path) -> Vec<Judge> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };

    let mut judges = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md") {
            match parse_judge_file(&path) {
                Ok(judge) => judges.push(judge),
                Err(e) => {
                    tracing::warn!("failed to parse judge {}: {e:?}", path.display());
                }
            }
        }
    }

    judges.sort_by(|a, b| a.name.cmp(&b.name));
    judges
}

/// Scans both system and user judge directories, merging results.
///
/// System judges are loaded first. User judges with the same name
/// override system ones. Results are sorted by name.
pub fn scan_judges_merged(user_dir: &Path, system_dir: &Path) -> Vec<Judge> {
    let mut seen = std::collections::HashSet::new();
    let mut judges = Vec::new();

    // System judges first (lower priority).
    for judge in scan_judges_dir(system_dir) {
        seen.insert(judge.name.clone());
        judges.push(judge);
    }

    // User judges override system ones of the same name.
    for judge in scan_judges_dir(user_dir) {
        if seen.contains(&judge.name) {
            // Replace the system judge with the user version.
            if let Some(pos) = judges.iter().position(|j| j.name == judge.name) {
                judges[pos] = judge;
            }
        } else {
            seen.insert(judge.name.clone());
            judges.push(judge);
        }
    }

    judges.sort_by(|a, b| a.name.cmp(&b.name));
    judges
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use std::path::PathBuf;

    fn test_path(name: &str) -> PathBuf {
        PathBuf::from(format!("/test/{name}.md"))
    }

    #[rstest::rstest]
    fn parse_judge_with_valid_frontmatter() {
        // Given a valid judge file content.
        let content = "+++\nname = \"accuracy\"\ndescription = \"Checks accuracy\"\n+++\n\nEvaluate the session.\n";

        // When parsing.
        let judge = parse_judge_content(content, &test_path("accuracy")).expect("parse");

        // Then fields are correctly extracted.
        assert_eq!(judge.name, "accuracy");
        assert_eq!(judge.description, "Checks accuracy");
        assert_eq!(judge.body, "Evaluate the session.");
        assert!(judge.model.is_none());
    }

    #[rstest::rstest]
    fn parse_judge_with_model_override() {
        // Given a judge file with a model override.
        let content =
            "+++\nname = \"fast-check\"\nmodel = \"anthropic/claude-haiku\"\n+++\n\nQuick check.";

        // When parsing.
        let judge = parse_judge_content(content, &test_path("fast-check")).expect("parse");

        // Then model is set.
        assert_eq!(judge.model.as_deref(), Some("anthropic/claude-haiku"));
    }

    #[rstest::rstest]
    fn parse_judge_without_description() {
        // Given a judge file without description (uses default).
        let content = "+++\nname = \"minimal\"\n+++\n\nBody text here.";

        // When parsing.
        let judge = parse_judge_content(content, &test_path("minimal")).expect("parse");

        // Then description defaults to empty string.
        assert_eq!(judge.name, "minimal");
        assert_eq!(judge.description, "");
        assert_eq!(judge.body, "Body text here.");
    }

    #[rstest::rstest]
    fn parse_judge_fails_without_frontmatter() {
        // Given content without +++ delimiter.
        let content = "Just some text without frontmatter.";

        // When parsing.
        let result = parse_judge_content(content, &test_path("bad"));

        // Then it fails with Frontmatter error.
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().current_context(),
            JudgeParseError::Frontmatter
        ));
    }

    #[rstest::rstest]
    fn parse_judge_fails_without_closing_delimiter() {
        // Given content with opening +++ but no closing +++.
        let content = "+++\nname = \"test\"\nNo closing delimiter here.";

        // When parsing.
        let result = parse_judge_content(content, &test_path("bad"));

        // Then it fails with Frontmatter error.
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().current_context(),
            JudgeParseError::Frontmatter
        ));
    }

    #[rstest::rstest]
    fn parse_judge_fails_with_invalid_toml() {
        // Given content with invalid TOML in frontmatter.
        let content = "+++\nname = invalid toml\n+++\n\nBody.";

        // When parsing.
        let result = parse_judge_content(content, &test_path("bad"));

        // Then it fails with Parse error.
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().current_context(),
            JudgeParseError::Parse
        ));
    }

    #[rstest::rstest]
    fn scan_judges_dir_returns_sorted_judges() {
        // Given a directory with judge files.
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
        let judges = scan_judges_dir(dir.path());

        // Then judges are sorted by name.
        assert_eq!(judges.len(), 2);
        assert_eq!(judges[0].name, "alpha");
        assert_eq!(judges[1].name, "beta");
    }

    #[rstest::rstest]
    fn scan_judges_dir_returns_empty_for_missing_dir() {
        // Given a nonexistent directory.
        let dir = PathBuf::from("/nonexistent/path");

        // When scanning.
        let judges = scan_judges_dir(&dir);

        // Then an empty vec is returned.
        assert!(judges.is_empty());
    }

    #[rstest::rstest]
    fn scan_judges_dir_skips_invalid_files() {
        // Given a directory with one valid and one invalid file.
        let dir = tempfile::TempDir::new().expect("temp dir");

        let valid = dir.path().join("valid.md");
        std::fs::write(
            &valid,
            "+++\nname = \"valid\"\ndescription = \"V\"\n+++\n\nValid body.",
        )
        .expect("write");

        let invalid = dir.path().join("invalid.md");
        std::fs::write(&invalid, "Not a valid judge file.").expect("write");

        // When scanning.
        let judges = scan_judges_dir(dir.path());

        // Then only the valid judge is returned.
        assert_eq!(judges.len(), 1);
        assert_eq!(judges[0].name, "valid");
    }

    #[rstest::rstest]
    fn scan_judges_dir_ignores_non_md_files() {
        // Given a directory with a .txt file.
        let dir = tempfile::TempDir::new().expect("temp dir");

        let txt = dir.path().join("notes.txt");
        std::fs::write(
            &txt,
            "+++\nname = \"hidden\"\ndescription = \"H\"\n+++\n\nBody.",
        )
        .expect("write");

        // When scanning.
        let judges = scan_judges_dir(dir.path());

        // Then no judges are found.
        assert!(judges.is_empty());
    }

    #[rstest::rstest]
    fn scan_judges_merged_user_overrides_system() {
        // Given system and user dirs both with a judge named "accuracy".
        let sys = tempfile::TempDir::new().expect("temp dir");
        let usr = tempfile::TempDir::new().expect("temp dir");

        std::fs::write(
            sys.path().join("accuracy.md"),
            "+++\nname = \"accuracy\"\ndescription = \"System\"\n+++\n\nSystem body.",
        )
        .expect("write");

        std::fs::write(
            usr.path().join("accuracy.md"),
            "+++\nname = \"accuracy\"\ndescription = \"User\"\n+++\n\nUser body.",
        )
        .expect("write");

        // When merging.
        let judges = scan_judges_merged(usr.path(), sys.path());

        // Then the user version wins.
        assert_eq!(judges.len(), 1);
        assert_eq!(judges[0].description, "User");
    }

    #[rstest::rstest]
    fn scan_judges_merged_combines_unique_names() {
        // Given system has "accuracy", user has "style".
        let sys = tempfile::TempDir::new().expect("temp dir");
        let usr = tempfile::TempDir::new().expect("temp dir");

        std::fs::write(
            sys.path().join("accuracy.md"),
            "+++\nname = \"accuracy\"\ndescription = \"A\"\n+++\n\nBody.",
        )
        .expect("write");

        std::fs::write(
            usr.path().join("style.md"),
            "+++\nname = \"style\"\ndescription = \"S\"\n+++\n\nBody.",
        )
        .expect("write");

        // When merging.
        let judges = scan_judges_merged(usr.path(), sys.path());

        // Then both are present, sorted by name.
        assert_eq!(judges.len(), 2);
        assert_eq!(judges[0].name, "accuracy");
        assert_eq!(judges[1].name, "style");
    }
}
