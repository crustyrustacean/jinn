//! User preferences data type and file I/O.
//!
//! Defines [`UserPreferences`] as the schema for `nullslop.toml`,
//! along with loading and saving logic. The file lives at
//! `~/.config/nullslop/nullslop.toml` and is auto-created on first save.

use std::path::{Path, PathBuf};

use crate::common::app_info::{APP_NAME, PREFS_FILE_NAME};
use error_stack::{Report, ResultExt as _};
use serde::{Deserialize, Serialize};
use wherror::Error;

/// Errors that can occur during user preferences I/O.
#[derive(Debug, Error)]
pub enum UserPreferencesError {
    /// Filesystem I/O failure.
    #[error("user preferences I/O error")]
    Io,
    /// TOML parsing or structural error.
    #[error("user preferences parse error")]
    Parse,
}

/// User preferences persisted in `nullslop.toml`.
///
/// This file stores user behavior preferences that should survive
/// app restarts — e.g., the last model and strategy selected from pickers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserPreferences {
    /// The provider ID of the last model selected from the model picker.
    /// Format: `{provider_name}/{model}` (e.g., `"ollama/llama3"`).
    #[serde(default)]
    pub last_model: Option<String>,
    /// The strategy ID of the last strategy selected from the strategy picker.
    /// Format: strategy name (e.g., `"sliding_window"`).
    #[serde(default)]
    pub last_strategy: Option<String>,
    /// Maximum number of lines to display for tool result entries in the chat log.
    /// `None` means use the built-in default (5 lines).
    #[serde(default)]
    pub tool_result_max_lines: Option<u16>,
}

/// Returns the path to the user preferences file.
///
/// Uses `dirs::config_dir()` → `~/.config/nullslop/nullslop.toml`.
#[must_use]
pub fn preferences_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_NAME)
        .join(PREFS_FILE_NAME)
}

/// Loads user preferences from the default path.
///
/// Returns default preferences if the file does not exist.
///
/// # Errors
///
/// Returns [`UserPreferencesError::Parse`] if the TOML is malformed.
/// Returns [`UserPreferencesError::Io`] if the file cannot be read.
pub fn load_preferences() -> Result<UserPreferences, Report<UserPreferencesError>> {
    load_preferences_from(preferences_path())
}

/// Loads preferences from a specific path.
pub(crate) fn load_preferences_from<P>(
    path: P,
) -> Result<UserPreferences, Report<UserPreferencesError>>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();

    if !path.exists() {
        return Ok(UserPreferences::default());
    }

    let content = std::fs::read_to_string(path)
        .change_context(UserPreferencesError::Io)
        .attach("failed to read user preferences")?;

    toml::from_str(&content)
        .change_context(UserPreferencesError::Parse)
        .attach("failed to parse user preferences")
}

/// Saves preferences to the default path.
///
/// Creates parent directories as needed.
///
/// # Errors
///
/// Returns [`UserPreferencesError::Parse`] if serialization fails.
/// Returns [`UserPreferencesError::Io`] if writing fails.
pub fn save_preferences(prefs: &UserPreferences) -> Result<(), Report<UserPreferencesError>> {
    save_preferences_to(prefs, preferences_path())
}

/// Saves preferences to a specific path.
pub(crate) fn save_preferences_to<P>(
    prefs: &UserPreferences,
    path: P,
) -> Result<(), Report<UserPreferencesError>>
where
    P: AsRef<Path>,
{
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent)
            .change_context(UserPreferencesError::Io)
            .attach("failed to create preferences directory")?;
    }

    let content = toml::to_string_pretty(prefs)
        .change_context(UserPreferencesError::Parse)
        .attach("failed to serialize user preferences")?;

    std::fs::write(path.as_ref(), content)
        .change_context(UserPreferencesError::Io)
        .attach("failed to write user preferences")
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[rstest::rstest]
    fn default_preferences_has_no_last_model() {
        // Given default preferences.
        let prefs = UserPreferences::default();

        // Then last_model, last_strategy, and tool_result_max_lines are None.
        assert!(prefs.last_model.is_none());
        assert!(prefs.last_strategy.is_none());
        assert!(prefs.tool_result_max_lines.is_none());
    }

    #[rstest::rstest]
    fn load_returns_default_when_file_missing() {
        // Given a path to a nonexistent file.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then defaults are returned.
        assert!(prefs.last_model.is_none());
        assert!(prefs.last_strategy.is_none());
        assert!(prefs.tool_result_max_lines.is_none());
    }

    #[rstest::rstest]
    fn save_then_load_round_trips() {
        // Given preferences with a last_model and last_strategy.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            last_model: Some("ollama/llama3".to_owned()),
            last_strategy: Some("sliding_window".to_owned()),
            tool_result_max_lines: None,
        };

        // When saving and reloading.
        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");

        // Then the round-tripped data matches.
        assert_eq!(reloaded.last_model.as_deref(), Some("ollama/llama3"));
        assert_eq!(reloaded.last_strategy.as_deref(), Some("sliding_window"));
    }

    #[rstest::rstest]
    fn load_parses_toml_content() {
        // Given a TOML file with last_model and last_strategy.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"last_model = "openrouter/anthropic/claude-sonnet-4-20250514"
last_strategy = "sliding_window""#,
        )
        .expect("write");

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then last_model and last_strategy are parsed.
        assert_eq!(
            prefs.last_model.as_deref(),
            Some("openrouter/anthropic/claude-sonnet-4-20250514")
        );
        assert_eq!(prefs.last_strategy.as_deref(), Some("sliding_window"));
    }

    #[rstest::rstest]
    fn load_handles_empty_file() {
        // Given an empty TOML file.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, "").expect("write");

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then defaults are returned (all fields None).
        assert!(prefs.last_model.is_none());
        assert!(prefs.last_strategy.is_none());
        assert!(prefs.tool_result_max_lines.is_none());
    }

    #[rstest::rstest]
    fn save_creates_parent_directories() {
        // Given a nested path that doesn't exist.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("nested").join("dir").join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            last_model: Some("test/model".to_owned()),
            last_strategy: None,
            tool_result_max_lines: None,
        };

        // When saving.
        save_preferences_to(&prefs, &path).expect("save");

        // Then the file exists.
        assert!(path.exists());
    }

    #[rstest::rstest]
    fn save_then_load_round_trips_tool_result_max_lines() {
        // Given preferences with a tool_result_max_lines override.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            last_model: None,
            last_strategy: None,
            tool_result_max_lines: Some(10),
        };

        // When saving and reloading.
        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");

        // Then the round-tripped value matches.
        assert_eq!(reloaded.tool_result_max_lines, Some(10));
    }

    #[rstest::rstest]
    fn preferences_path_ends_with_nullslop_toml() {
        // Given the standard preferences path.
        let path = preferences_path();

        // Then it ends with nullslop/nullslop.toml.
        assert!(path.to_string_lossy().ends_with("nullslop/nullslop.toml"));
    }
}
