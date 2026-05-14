//! Prompt template data model, loading, and lookup.
//!
//! Templates are markdown files with TOML frontmatter (delimited by `+++`).
//! This crate provides types for loading, storing, and searching templates
//! independently of the application bus or state.
//!
//! The [`PromptTemplate`] data struct itself lives in `nullslop-domain` so it
//! can travel across the actor boundary. This crate owns the loading, parsing,
//! and storage logic.

mod expand;
mod loader;
mod store;

pub use expand::expand_tokens;
pub use loader::PromptTemplateParseError;
pub use loader::render_template_file;
pub use store::PromptTemplateStore;
pub use store::PromptTemplateStoreError;

use std::path::{Path, PathBuf};

use crate::common::app_info::APP_NAME;
use crate::protocol::PromptTemplate;
use error_stack::{Report, ResultExt as _};
use wherror::Error;

/// Errors that can occur when ensuring the prompts directory and example file exist.
#[derive(Debug, Error)]
#[error(debug)]
pub enum EnsureExampleError {
    /// Filesystem I/O failure.
    Io,
}

/// File name for the example prompt template.
const EXAMPLE_FILENAME: &str = "example.md";

/// Returns the default path to the prompt templates directory.
///
/// Uses `dirs::config_dir()` → `~/.config/nullslop/prompts/`.
/// Primarily for startup — actors receive their path via injection.
#[must_use]
pub fn prompts_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_NAME)
        .join("prompts")
}

/// Ensures the prompts directory exists and contains an example template file.
///
/// Creates `~/.config/nullslop/prompts/` if missing, then writes an example
/// template generated from a [`PromptTemplate`] struct so the file format
/// stays in sync with the frontmatter schema as fields are added.
///
/// If the example file already exists, does nothing (no overwrite).
///
/// # Errors
///
/// Returns an error if the directory or file cannot be created due to I/O failure.
pub fn ensure_prompts_dir_with_example() -> Result<(), Report<EnsureExampleError>> {
    let dir = prompts_dir();
    ensure_prompts_dir_with_example_to(&dir)
}

/// Testable version that writes to an explicit directory.
pub(crate) fn ensure_prompts_dir_with_example_to(
    dir: &Path,
) -> Result<(), Report<EnsureExampleError>> {
    std::fs::create_dir_all(dir)
        .change_context(EnsureExampleError::Io)
        .attach("failed to create prompts directory")?;

    let example_path = dir.join(EXAMPLE_FILENAME);

    if example_path.exists() {
        return Ok(());
    }

    let template = PromptTemplate {
        name: "example".to_owned(),
        description: "An example prompt template — edit or delete me".to_owned(),
        body: "You are a helpful assistant.".to_owned(),
    };

    let content = loader::render_template_file(&template);

    std::fs::write(&example_path, content)
        .change_context(EnsureExampleError::Io)
        .attach(format!("failed to write {}", example_path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[rstest::rstest]
    fn ensure_creates_directory() {
        // Given a nonexistent directory.
        let dir = TempDir::new().expect("temp dir");
        let prompts = dir.path().join("prompts");
        assert!(!prompts.exists());

        // When ensuring the prompts dir with example.
        ensure_prompts_dir_with_example_to(&prompts).expect("ensure");

        // Then the directory exists.
        assert!(prompts.exists());
    }

    #[rstest::rstest]
    fn ensure_creates_example_file() {
        // Given a nonexistent directory.
        let dir = TempDir::new().expect("temp dir");
        let prompts = dir.path().join("prompts");

        // When ensuring the prompts dir with example.
        ensure_prompts_dir_with_example_to(&prompts).expect("ensure");

        // Then the example file exists.
        let example = prompts.join(EXAMPLE_FILENAME);
        assert!(example.exists());
    }

    #[rstest::rstest]
    fn example_file_is_valid_template() {
        // Given a nonexistent directory.
        let dir = TempDir::new().expect("temp dir");
        let prompts = dir.path().join("prompts");

        // When ensuring the prompts dir with example.
        ensure_prompts_dir_with_example_to(&prompts).expect("ensure");

        // Then the file parses back to a valid template.
        let example = prompts.join(EXAMPLE_FILENAME);
        let content = std::fs::read_to_string(&example).expect("read");
        let template =
            crate::feat::context::prompt_template::loader::parse_template_content(&content)
                .expect("parse");
        assert_eq!(template.name, "example");
    }

    #[rstest::rstest]
    fn ensure_does_not_overwrite_existing_file() {
        // Given a directory that already has an example file.
        let dir = TempDir::new().expect("temp dir");
        let prompts = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts).expect("create dir");
        std::fs::write(prompts.join(EXAMPLE_FILENAME), "custom content").expect("write");

        // When ensuring the prompts dir with example.
        ensure_prompts_dir_with_example_to(&prompts).expect("ensure");

        // Then the existing file is not overwritten.
        let content = std::fs::read_to_string(prompts.join(EXAMPLE_FILENAME)).expect("read");
        assert_eq!(content, "custom content");
    }

    #[rstest::rstest]
    fn ensure_is_idempotent() {
        // Given a prompts directory.
        let dir = TempDir::new().expect("temp dir");
        let prompts = dir.path().join("prompts");

        // When ensuring twice.
        ensure_prompts_dir_with_example_to(&prompts).expect("first ensure");
        ensure_prompts_dir_with_example_to(&prompts).expect("second ensure");

        // Then exactly one example file exists with valid content.
        let example = prompts.join(EXAMPLE_FILENAME);
        let content = std::fs::read_to_string(&example).expect("read");
        let template =
            crate::feat::context::prompt_template::loader::parse_template_content(&content)
                .expect("parse");
        assert_eq!(template.name, "example");
    }
}
