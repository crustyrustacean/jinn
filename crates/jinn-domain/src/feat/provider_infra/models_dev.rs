//! models.dev reference data loader.
//!
//! Loads a models.dev JSON dump from disk (user cache or system fallback),
//! parses it into a flat lookup table of model ID → context length, and
//! provides efficient lookup for filling in missing `context_length` values
//! during model discovery and cache merge.

use std::collections::HashMap;
use std::path::Path;

/// A lookup table mapping model IDs to their context window size in tokens.
///
/// Loaded from the models.dev reference file at app startup. Used as a
/// fallback when provider APIs and `providers.toml` don't supply a value.
#[derive(Debug, Clone, Default)]
pub struct ModelsDevData {
    /// Model ID → context length in tokens.
    pub(crate) context_lengths: HashMap<String, u32>,
}

impl ModelsDevData {
    /// Creates a new, empty lookup table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads models.dev reference data from disk.
    ///
    /// Checks the user cache path first, then falls back to the system path.
    /// If neither file exists, returns an empty lookup. If the user file is
    /// corrupted (invalid JSON), falls back to the system file with a warning.
    ///
    /// This never returns an error — missing or unreadable files result in
    /// an empty lookup table.
    pub fn load(user_path: &Path, system_path: &Path) -> Self {
        // Try user path first.
        if user_path.exists() {
            match Self::parse_file(user_path) {
                Ok(data) => return data,
                Err(e) => {
                    tracing::warn!(
                        path = %user_path.display(),
                        err = %e,
                        "corrupted models.dev user file, trying system fallback"
                    );
                }
            }
        }

        // Fall back to system path.
        if system_path.exists() {
            match Self::parse_file(system_path) {
                Ok(data) => return data,
                Err(e) => {
                    tracing::warn!(
                        path = %system_path.display(),
                        err = %e,
                        "corrupted models.dev system file"
                    );
                }
            }
        }

        // Neither file exists or both corrupted — return empty.
        Self::new()
    }

    /// Looks up the context length for a model ID.
    ///
    /// Returns `None` if the model is not in the reference data.
    #[must_use]
    pub fn get(&self, model_id: &str) -> Option<u32> {
        self.context_lengths.get(model_id).copied()
    }

    /// Returns `true` if the lookup table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.context_lengths.is_empty()
    }

    /// Returns the number of models in the lookup table.
    #[must_use]
    pub fn len(&self) -> usize {
        self.context_lengths.len()
    }

    /// Parses a models.dev JSON file and builds the lookup table.
    fn parse_file(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path).map_err(|e| format!("failed to read: {e}"))?;

        let providers: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&content).map_err(|e| format!("invalid JSON: {e}"))?;

        let mut context_lengths = HashMap::new();

        for (_provider_name, provider_data) in &providers {
            let Some(models) = provider_data.get("models").and_then(|m| m.as_object()) else {
                continue;
            };
            for (model_id, model_data) in models {
                // Extract limit.context if present.
                let ctx_len = model_data
                    .get("limit")
                    .and_then(|limit| limit.get("context"))
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|c| u32::try_from(c).ok());

                if let Some(ctx) = ctx_len {
                    // Only insert if not already present (first provider wins).
                    context_lengths.entry(model_id.clone()).or_insert(ctx);
                }
            }
        }

        Ok(Self { context_lengths })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;

    fn write_json(dir: &std::path::Path, filename: &str, json: &str) -> std::path::PathBuf {
        let path = dir.join(filename);
        std::fs::write(&path, json).expect("write json");
        path
    }

    fn minimal_models_dev_json() -> &'static str {
        r#"{
            "openai": {
                "models": {
                    "gpt-4o": {
                        "name": "GPT-4o",
                        "limit": { "context": 128000, "output": 16384 }
                    },
                    "gpt-3.5-turbo": {
                        "name": "GPT-3.5 Turbo",
                        "limit": { "context": 16385, "output": 4096 }
                    }
                }
            },
            "anthropic": {
                "models": {
                    "claude-sonnet-4-20250514": {
                        "name": "Claude Sonnet 4",
                        "limit": { "context": 200000, "output": 64000 }
                    }
                }
            },
            "zai": {
                "models": {
                    "glm-5.1": {
                        "name": "GLM 5.1",
                        "limit": { "context": 200000, "output": 131072 }
                    }
                }
            }
        }"#
    }

    #[rstest::rstest]
    fn load_parses_context_length_correctly() {
        // Given a valid models.dev file with multiple providers and models.
        let dir = tempfile::tempdir().expect("temp dir");
        let user_path = write_json(dir.path(), "models.dev.json", minimal_models_dev_json());

        // When loading.
        let data = ModelsDevData::load(&user_path, Path::new("/nonexistent"));

        // Then all model IDs map to correct context lengths.
        assert_eq!(data.get("gpt-4o"), Some(128_000));
        assert_eq!(data.get("gpt-3.5-turbo"), Some(16_385));
        assert_eq!(data.get("claude-sonnet-4-20250514"), Some(200_000));
        assert_eq!(data.get("glm-5.1"), Some(200_000));
        assert_eq!(data.len(), 4);
    }

    #[rstest::rstest]
    fn load_prefers_user_over_system() {
        // Given different data in user and system files.
        let user_dir = tempfile::tempdir().expect("temp dir");
        let system_dir = tempfile::tempdir().expect("temp dir");
        let user_path = write_json(
            user_dir.path(),
            "models.dev.json",
            r#"{"openai":{"models":{"gpt-4o":{"limit":{"context":100}}}}}"#,
        );
        write_json(
            system_dir.path(),
            "models.dev.json",
            r#"{"openai":{"models":{"gpt-4o":{"limit":{"context":200}}}}}"#,
        );

        // When loading.
        let data = ModelsDevData::load(&user_path, &system_dir.path().join("models.dev.json"));

        // Then the user file value wins.
        assert_eq!(data.get("gpt-4o"), Some(100));
    }

    #[rstest::rstest]
    fn load_falls_back_to_system_when_user_missing() {
        // Given only a system file.
        let system_dir = tempfile::tempdir().expect("temp dir");
        let system_path = write_json(
            system_dir.path(),
            "models.dev.json",
            r#"{"openai":{"models":{"gpt-4o":{"limit":{"context":200}}}}}"#,
        );

        // When loading with no user file.
        let data = ModelsDevData::load(Path::new("/nonexistent"), &system_path);

        // Then system data is used.
        assert_eq!(data.get("gpt-4o"), Some(200));
    }

    #[rstest::rstest]
    fn load_returns_empty_when_neither_exists() {
        // Given no files at either path.
        let data = ModelsDevData::load(
            Path::new("/nonexistent/user"),
            Path::new("/nonexistent/system"),
        );

        // Then the lookup is empty (no panic, no error).
        assert!(data.is_empty());
        assert_eq!(data.len(), 0);
    }

    #[rstest::rstest]
    fn load_handles_missing_limit_field() {
        // Given a model entry with no limit key.
        let dir = tempfile::tempdir().expect("temp dir");
        let user_path = write_json(
            dir.path(),
            "models.dev.json",
            r#"{"openai":{"models":{"gpt-4o":{"name":"GPT-4o"}}}}"#,
        );

        // When loading.
        let data = ModelsDevData::load(&user_path, Path::new("/nonexistent"));

        // Then the model ID is absent from the lookup table.
        assert_eq!(data.get("gpt-4o"), None);
        assert!(data.is_empty());
    }

    #[rstest::rstest]
    fn load_handles_zero_context() {
        // Given a model with context: 0 (e.g., embedding model).
        let dir = tempfile::tempdir().expect("temp dir");
        let user_path = write_json(
            dir.path(),
            "models.dev.json",
            r#"{"openai":{"models":{"text-embedding-3-large":{"limit":{"context":0}}}}}"#,
        );

        // When loading.
        let data = ModelsDevData::load(&user_path, Path::new("/nonexistent"));

        // Then get() returns Some(0) — not None.
        assert_eq!(data.get("text-embedding-3-large"), Some(0));
    }

    #[rstest::rstest]
    fn load_handles_corrupted_user_file() {
        // Given garbage in user path and valid data in system path.
        let user_dir = tempfile::tempdir().expect("temp dir");
        let system_dir = tempfile::tempdir().expect("temp dir");
        let user_path = user_dir.path().join("models.dev.json");
        std::fs::write(&user_path, "NOT VALID JSON!!!").expect("write garbage");
        let system_path = write_json(
            system_dir.path(),
            "models.dev.json",
            r#"{"openai":{"models":{"gpt-4o":{"limit":{"context":200}}}}}"#,
        );

        // When loading.
        let data = ModelsDevData::load(&user_path, &system_path);

        // Then system data is used.
        assert_eq!(data.get("gpt-4o"), Some(200));
    }

    #[rstest::rstest]
    fn first_provider_wins_for_duplicate_model_ids() {
        // Given two providers listing the same model ID with different contexts.
        let dir = tempfile::tempdir().expect("temp dir");
        let user_path = write_json(
            dir.path(),
            "models.dev.json",
            r#"{
                "zai": {
                    "models": {
                        "glm-5.1": { "limit": { "context": 200000 } }
                    }
                },
                "zai-coding-plan": {
                    "models": {
                        "glm-5.1": { "limit": { "context": 999999 } }
                    }
                }
            }"#,
        );

        // When loading.
        let data = ModelsDevData::load(&user_path, Path::new("/nonexistent"));

        // Then the first provider's value is used (insert wins).
        assert_eq!(data.get("glm-5.1"), Some(200_000));
    }

    #[rstest::rstest]
    fn get_returns_none_for_unknown_model() {
        // Given an empty lookup table.
        let data = ModelsDevData::new();

        // When looking up a model that doesn't exist.
        let result = data.get("my-custom-llama");

        // Then None is returned.
        assert_eq!(result, None);
    }

    // --- S-Tier: Kill mutant for is_empty ---

    #[rstest::rstest]
    fn is_empty_returns_false_when_data_present() {
        // Kills: replace is_empty with true.
        // If is_empty always returned true, data loading would appear to never work.
        let dir = tempfile::tempdir().expect("temp dir");
        let user_path = write_json(
            dir.path(),
            "models.dev.json",
            r#"{"openai":{"models":{"gpt-4o":{"limit":{"context":128000}}}}}"#,
        );

        let data = ModelsDevData::load(&user_path, Path::new("/nonexistent"));

        assert!(!data.is_empty(), "is_empty should return false when data is loaded");
        assert_eq!(data.get("gpt-4o"), Some(128_000));
    }
}
