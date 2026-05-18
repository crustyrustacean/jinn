//! Model cache — persisted discovery results for provider models.
//!
//! [`ModelCache`] stores the results of model discovery (provider name → list of
//! model metadata) as a JSON file on disk. It is loaded after a refresh completes
//! and read by the UI to display "last updated" information.
//!
//! Backward compatibility: the cache can load the old format where entries were
//! `HashMap<String, Vec<String>>` (just model IDs) by converting each string to
//! a `ModelInfo` with `context_length: None`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::common::app_info::APP_NAME;
use error_stack::{Report, ResultExt as _};
use jiff::Timestamp;
use nullslop_provider::ModelInfo;
use serde::{Deserialize, Serialize};
use wherror::Error;

/// Errors that can occur during model cache I/O.
#[derive(Debug, Error)]
#[error(debug)]
pub struct ModelCacheError;

/// Persisted model discovery results.
///
/// Maps provider names to the list of discovered model metadata,
/// along with the timestamp of the last successful refresh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCache {
    /// Provider name → list of discovered model metadata.
    pub entries: HashMap<String, Vec<ModelInfo>>,

    /// When the cache was last written to disk (UTC).
    /// `None` for caches created before this field existed.
    pub last_updated_at: Option<Timestamp>,
}

/// Intermediate format for backward-compatible deserialization.
///
/// Old caches stored `Vec<String>` (just model IDs). This struct accepts both
/// the old and new formats by attempting to deserialize as the new format first,
/// then falling back to converting strings.
#[derive(Deserialize)]
struct LegacyModelCache {
    #[serde(deserialize_with = "deserialize_model_entries")]
    entries: HashMap<String, Vec<ModelInfo>>,
    last_updated_at: Option<Timestamp>,
}

/// Deserializes model entries, accepting both `Vec<ModelInfo>` (new) and
/// `Vec<String>` (legacy) formats. String entries are converted to `ModelInfo`
/// with `context_length: None`.
fn deserialize_model_entries<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, Vec<ModelInfo>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    // Try the new format first (Vec<ModelInfo>).
    let value = serde_json::Value::deserialize(deserializer)?;

    let map: HashMap<String, Vec<ModelInfo>> = if let Ok(m) = serde_json::from_value(value.clone())
    {
        m
    } else {
        // Fall back to legacy format (Vec<String>).
        let legacy: HashMap<String, Vec<String>> = serde_json::from_value(value)
            .map_err(|e| Error::custom(format!("failed to parse model entries: {e}")))?;
        legacy
            .into_iter()
            .map(|(k, ids)| {
                let infos = ids
                    .into_iter()
                    .map(|id| ModelInfo {
                        id,
                        context_length: None,
                    })
                    .collect();
                (k, infos)
            })
            .collect()
    };

    Ok(map)
}

impl ModelCache {
    /// Creates a new, empty model cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            last_updated_at: None,
        }
    }

    /// Loads the model cache from disk.
    ///
    /// Returns `Ok(None)` if the file does not exist.
    /// Returns `Ok(Some(cache))` if the file was loaded and parsed successfully.
    /// Supports both the new format (`Vec<ModelInfo>`) and the legacy format
    /// (`Vec<String>`).
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load(path: &Path) -> Result<Option<Self>, Report<ModelCacheError>> {
        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(path)
            .change_context(ModelCacheError)
            .attach("failed to read model cache file")?;

        // Try new format first, fall back to legacy deserializer.
        let cache: Self = if let Ok(c) = serde_json::from_str(&content) {
            c
        } else {
            let legacy: LegacyModelCache = serde_json::from_str(&content)
                .change_context(ModelCacheError)
                .attach("failed to parse model cache file")?;
            Self {
                entries: legacy.entries,
                last_updated_at: legacy.last_updated_at,
            }
        };

        Ok(Some(cache))
    }

    /// Saves the model cache to disk.
    ///
    /// Creates parent directories if they do not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), Report<ModelCacheError>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .change_context(ModelCacheError)
                .attach("failed to create model cache directory")?;
        }

        let content = serde_json::to_string_pretty(self)
            .change_context(ModelCacheError)
            .attach("failed to serialize model cache")?;

        std::fs::write(path, content)
            .change_context(ModelCacheError)
            .attach("failed to write model cache file")?;

        Ok(())
    }
}

impl Default for ModelCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the path to the model cache file.
///
/// Uses `dirs::cache_dir()` → `~/.cache/nullslop/model_cache.json`.
#[must_use]
pub fn cache_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_NAME)
        .join("model_cache.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn new_cache_is_empty() {
        // Given a new ModelCache.
        let cache = ModelCache::new();

        // Then it has no entries and no timestamp.
        assert!(cache.entries.is_empty());
        assert!(cache.last_updated_at.is_none());
    }

    #[rstest::rstest]
    fn loaded_matches_original_entries() {
        // Given a cache with entries and a timestamp.
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "ollama".to_owned(),
            vec![
                ModelInfo {
                    id: "llama3".to_owned(),
                    context_length: Some(8192),
                },
                ModelInfo {
                    id: "mistral".to_owned(),
                    context_length: None,
                },
            ],
        );
        cache.last_updated_at = Some(Timestamp::now());

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("model_cache.json");

        // When saving and loading.
        cache.save(&path).expect("save");
        let loaded = ModelCache::load(&path).expect("load");

        // Then the loaded cache entries match the original.
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries["ollama"].len(), 2);
        assert_eq!(loaded.entries["ollama"][0].id, "llama3");
        assert_eq!(loaded.entries["ollama"][0].context_length, Some(8192));
    }

    #[rstest::rstest]
    fn load_accepts_legacy_string_array_format() {
        // Given a cache JSON file with old string-array format.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("model_cache.json");
        let json = r#"{ "entries": { "ollama": ["llama3", "mistral"] } }"#;
        std::fs::write(&path, json).expect("write");

        // When loading.
        let loaded = ModelCache::load(&path).expect("load");

        // Then it deserializes, converting strings to ModelInfo with no context_length.
        assert!(loaded.is_some());
        let cache = loaded.unwrap();
        assert_eq!(cache.entries["ollama"].len(), 2);
        assert_eq!(cache.entries["ollama"][0].id, "llama3");
        assert_eq!(cache.entries["ollama"][0].context_length, None);
        assert_eq!(cache.entries["ollama"][1].id, "mistral");
        assert!(cache.last_updated_at.is_none());
    }

    #[rstest::rstest]
    fn load_accepts_cache_without_timestamp() {
        // Given a cache JSON file without last_updated_at.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("model_cache.json");
        let json = r#"{ "entries": { "ollama": [{"id": "llama3", "context_length": 8192}] } }"#;
        std::fs::write(&path, json).expect("write");

        // When loading.
        let loaded = ModelCache::load(&path).expect("load");

        // Then it deserializes with last_updated_at = None.
        assert!(loaded.is_some());
        let cache = loaded.unwrap();
        assert_eq!(cache.entries["ollama"].len(), 1);
        assert_eq!(cache.entries["ollama"][0].context_length, Some(8192));
        assert!(cache.last_updated_at.is_none());
    }

    #[rstest::rstest]
    fn load_returns_none_when_file_missing() {
        // Given a path to a nonexistent file.
        let path = PathBuf::from("/tmp/nullslop_test_nonexistent_cache.json");

        // When loading.
        let result = ModelCache::load(&path);

        // Then Ok(None) is returned.
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}
