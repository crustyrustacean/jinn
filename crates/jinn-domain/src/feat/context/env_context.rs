//! Environment context builder - assembles the default system prompt for the LLM.
//!
//! Combines the active persona, project context files (AGENTS.md/CLAUDE.md),
//! current date, and working directory into a single string that is injected
//! as a `LlmMessage::System` at the front of every assembled prompt.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use crate::feat::persona::Persona;

/// Candidates for project context files, checked in order.
const CONTEXT_FILE_CANDIDATES: &[&str] = &["AGENTS.md", "AGENTS.MD", "CLAUDE.md", "CLAUDE.MD"];

/// Builds the environment context string for the system prompt.
///
/// Combines (in order):
/// 1. Active persona body (agent identity + guidelines)
/// 2. Project context files from CWD and ancestor directories
/// 3. Current date (YYYY-MM-DD)
/// 4. Current working directory
#[must_use]
pub fn build_env_context(
    persona: Option<&Persona>,
    context_files: &[ContextFile],
    cwd: &Path,
) -> String {
    let mut parts = Vec::new();

    // Persona body.
    if let Some(p) = persona
        && !p.body.is_empty()
    {
        parts.push(p.body.clone());
    }

    // Project context files.
    if !context_files.is_empty() {
        let mut section = String::from("\n\n# Project Context\n\n");
        section.push_str("Project-specific instructions and guidelines:\n\n");
        for file in context_files {
            let _ = write!(
                section,
                "## {}\n\n{}\n\n",
                file.path.display(),
                file.content
            );
        }
        parts.push(section);
    }

    // Date.
    let date = format_current_date();
    parts.push(format!("\nCurrent date: {date}"));

    // CWD.
    let cwd_str = cwd.to_string_lossy();
    parts.push(format!("Current working directory: {cwd_str}"));

    parts.join("")
}

/// A loaded project context file (e.g., AGENTS.md).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContextFile {
    /// The file path (for display in the prompt).
    pub path: PathBuf,
    /// The file content.
    pub content: String,
}

/// Loads project context files from the CWD and all ancestor directories.
///
/// Searches for AGENTS.md, AGENTS.MD, CLAUDE.md, CLAUDE.MD in each directory
/// from CWD up to root. Returns files ordered from root → CWD (so closer
/// files come last, matching pi-mono's ordering).
pub async fn load_project_context_files(cwd: &Path) -> Vec<ContextFile> {
    let mut files = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mut current = Some(cwd.to_path_buf());
    while let Some(dir) = current {
        if let Some(file) = load_context_file_from_dir(&dir).await {
            let canonical = file.path.clone();
            if seen.insert(canonical) {
                files.push(file);
            }
        }

        // Stop at root.
        if dir.parent().is_none() || dir.parent() == Some(dir.as_path()) {
            break;
        }
        current = dir.parent().map(std::path::Path::to_path_buf);
    }

    // Reverse so root files come first, CWD files come last.
    files.reverse();
    files
}

/// Loads a context file from a single directory (first match wins).
async fn load_context_file_from_dir(dir: &Path) -> Option<ContextFile> {
    for filename in CONTEXT_FILE_CANDIDATES {
        let path = dir.join(filename);
        if path.exists()
            && let Ok(content) = tokio::fs::read_to_string(&path).await
        {
            return Some(ContextFile { path, content });
        }
    }
    None
}

/// Synchronous version of [`load_project_context_files`].
///
/// Used when context rescan must happen outside an async runtime
/// (e.g., from the TUI suspend handler).
pub fn load_project_context_files_sync(cwd: &Path) -> Vec<ContextFile> {
    let mut files = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mut current = Some(cwd.to_path_buf());
    while let Some(dir) = current {
        if let Some(file) = load_context_file_from_dir_sync(&dir) {
            let canonical = file.path.clone();
            if seen.insert(canonical) {
                files.push(file);
            }
        }

        // Stop at root.
        if dir.parent().is_none() || dir.parent() == Some(dir.as_path()) {
            break;
        }
        current = dir.parent().map(std::path::Path::to_path_buf);
    }

    // Reverse so root files come first, CWD files come last.
    files.reverse();
    files
}

/// Synchronous single-directory context file loader.
fn load_context_file_from_dir_sync(dir: &Path) -> Option<ContextFile> {
    for filename in CONTEXT_FILE_CANDIDATES {
        let path = dir.join(filename);
        if path.exists()
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            return Some(ContextFile { path, content });
        }
    }
    None
}

/// Returns the current date as YYYY-MM-DD.
fn format_current_date() -> String {
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs() as i64;
    // Simple date calculation: days since epoch.
    let days = secs / 86400;
    // Gregorian calendar date from epoch (1970-01-01).
    date_from_days(days)
}

/// Converts days since Unix epoch to YYYY-MM-DD string.
fn date_from_days(total_days: i64) -> String {
    // Algorithm from Howard Hinnant.
    let z = total_days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
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

    fn test_persona(body: &str) -> Persona {
        Persona {
            name: "test".to_owned(),
            description: "Test persona".to_owned(),
            body: body.to_owned(),
            file_path: PathBuf::from("/test/persona.md"),
        }
    }

    #[rstest::rstest]
    fn build_env_context_includes_persona_body() {
        // Given a persona with body text.
        let persona = test_persona("You are a helpful assistant.");

        // When building env context.
        let result = build_env_context(Some(&persona), &[], Path::new("/project"));

        // Then the persona body is included.
        assert!(result.contains("You are a helpful assistant."));
    }

    #[rstest::rstest]
    fn build_env_context_includes_date_and_cwd() {
        // Given no persona.
        // When building env context.
        let result = build_env_context(None, &[], Path::new("/my/project"));

        // Then date and CWD are included.
        assert!(result.contains("Current date:"));
        assert!(result.contains("Current working directory: /my/project"));
    }

    #[rstest::rstest]
    fn build_env_context_includes_project_context_files() {
        // Given a context file.
        let files = vec![ContextFile {
            path: PathBuf::from("/project/AGENTS.md"),
            content: "# Style Guide\nUse Rust.".to_owned(),
        }];

        // When building env context.
        let result = build_env_context(None, &files, Path::new("/project"));

        // Then project context section is included.
        assert!(result.contains("# Project Context"));
        assert!(result.contains("/project/AGENTS.md"));
        assert!(result.contains("Use Rust."));
    }

    #[rstest::rstest]
    fn build_env_context_without_persona_still_has_date_and_cwd() {
        // Given no persona and no context files.
        // When building env context.
        let result = build_env_context(None, &[], Path::new("/project"));

        // Then date and CWD are still present.
        assert!(result.contains("Current date:"));
        assert!(result.contains("Current working directory: /project"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn load_project_context_files_finds_agents_md() {
        // Given a temp directory with AGENTS.md.
        let dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("AGENTS.md"), "# Test").expect("write");

        // When loading context files.
        let files = load_project_context_files(dir.path()).await;

        // Then the file is found.
        assert_eq!(files.len(), 1);
        assert!(files[0].path.ends_with("AGENTS.md"));
        assert_eq!(files[0].content, "# Test");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn load_project_context_files_returns_empty_for_missing() {
        // Given a directory with no context files.
        let dir = tempfile::TempDir::new().expect("temp dir");

        // When loading context files.
        let files = load_project_context_files(dir.path()).await;

        // Then no files are found.
        assert!(files.is_empty());
    }

    #[rstest::rstest]
    fn date_from_days_epoch() {
        // Given 0 days since epoch.
        // When converting.
        let date = date_from_days(0);

        // Then it's 1970-01-01.
        assert_eq!(date, "1970-01-01");
    }

    #[rstest::rstest]
    fn date_from_days_known_date() {
        // Given 20000 days since epoch (2024-10-04 roughly).
        let date = date_from_days(20000);

        // Then it's a valid date format.
        assert!(date.starts_with("20"));
        assert_eq!(date.len(), 10);
    }

    #[rstest::rstest]
    fn date_from_days_one_day() {
        // Given 1 day since epoch.
        // When converting.
        let date = date_from_days(1);

        // Then it's 1970-01-02.
        assert_eq!(date, "1970-01-02");
    }

    #[rstest::rstest]
    fn date_from_days_negative() {
        // Given -1 days (1969-12-31).
        let date = date_from_days(-1);

        // Then it's 1969-12-31.
        assert_eq!(date, "1969-12-31");
    }

    #[rstest::rstest]
    fn date_from_days_large_known_date() {
        // Given 20023 days = 2024-10-27 (verified externally).
        let date = date_from_days(20023);

        // Then it produces an exact known date.
        assert_eq!(date, "2024-10-27");
    }

    #[rstest::rstest]
    fn date_from_days_year_2000() {
        // Given 10957 days = 2000-01-01 (30 years including leap days).
        // 365 * 30 + 8 leap days (72,76,80,84,88,92,96,00 - 00 is leap in Gregorian) = 10957 + 7 = 10958?
        // Actually 10957 = 2000-01-01.
        let date = date_from_days(10957);

        // Then it starts with 2000.
        assert!(date.starts_with("2000-"));
    }

    #[rstest::rstest]
    fn format_current_date_is_not_empty() {
        // When formatting the current date.
        let date = format_current_date();

        // Then it is non-empty, has length 10, and is YYYY-MM-DD.
        assert_eq!(date.len(), 10);
        assert_eq!(date.chars().nth(4), Some('-'));
        assert_eq!(date.chars().nth(7), Some('-'));
    }

    #[rstest::rstest]
    fn format_current_date_is_not_xyzzy() {
        // When formatting the current date.
        let date = format_current_date();

        // Then it is not a placeholder string.
        assert_ne!(date, "xyzzy");
        assert_ne!(date, "");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn load_project_context_files_stops_at_root() {
        // Given a deeply nested temp dir with no context files anywhere.
        let root = tempfile::TempDir::new().expect("temp dir");
        let deep = root.path().join("a/b/c");
        std::fs::create_dir_all(&deep).expect("create");

        // When loading context files.
        let files = load_project_context_files(&deep).await;

        // Then no files are found (no AGENTS.md anywhere on path).
        assert!(files.is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn load_project_context_files_finds_file_in_parent() {
        // Given parent/AGENTS.md and child/ as CWD.
        let parent = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(parent.path().join("AGENTS.md"), "parent context").expect("write");
        let child = parent.path().join("subdir");
        std::fs::create_dir_all(&child).expect("create");

        // When loading from child.
        let files = load_project_context_files(&child).await;

        // Then the parent's file is found.
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].content, "parent context");
    }

    // --- Tests for load_project_context_files_sync ---

    #[rstest::rstest]
    fn sync_load_finds_agents_md() {
        // Given a temp directory with AGENTS.md.
        let dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("AGENTS.md"), "# Sync Test").expect("write");

        // When loading context files synchronously.
        let files = load_project_context_files_sync(dir.path());

        // Then the file is found.
        assert_eq!(files.len(), 1);
        assert!(files[0].path.ends_with("AGENTS.md"));
        assert_eq!(files[0].content, "# Sync Test");
    }

    #[rstest::rstest]
    fn sync_load_returns_empty_for_missing() {
        // Given a directory with no context files.
        let dir = tempfile::TempDir::new().expect("temp dir");

        // When loading context files synchronously.
        let files = load_project_context_files_sync(dir.path());

        // Then no files are found.
        assert!(files.is_empty());
    }

    #[rstest::rstest]
    fn sync_load_finds_file_in_parent() {
        // Given parent/AGENTS.md and child/ as CWD.
        let parent = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(parent.path().join("AGENTS.md"), "parent sync context").expect("write");
        let child = parent.path().join("subdir");
        std::fs::create_dir_all(&child).expect("create");

        // When loading from child synchronously.
        let files = load_project_context_files_sync(&child);

        // Then the parent's file is found.
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].content, "parent sync context");
    }
}
