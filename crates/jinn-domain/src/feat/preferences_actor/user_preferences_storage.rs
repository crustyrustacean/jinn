//! User preferences storage abstraction - trait for user preferences I/O.
//!
//! Defines [`UserPreferencesStorage`] as the abstraction for loading and saving
//! user preferences. [`FilesystemUserPreferencesStorage`] is the production
//! implementation; [`InMemoryUserPreferencesStorage`] is for testing.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use error_stack::{Report, ResultExt as _};
use parking_lot::RwLock;

use super::user_preferences::{UserPreferences, UserPreferencesError, preferences_path};

/// Trait for user preferences I/O.
///
/// Every external dependency must have a trait abstraction.
/// Filesystem I/O is an external dependency - this trait abstracts it so
/// tests can use in-memory storage instead of touching the real filesystem.
pub trait UserPreferencesStorage: Send + Sync + 'static {
    /// Returns the storage backend name (for debugging).
    fn name(&self) -> &'static str;

    /// Reloads user preferences from the underlying storage.
    ///
    /// Always hits the underlying storage (filesystem, in-memory, etc.) — bypassing any
    /// cache in the service wrapper. Returns default preferences if none exist.
    ///
    /// # Errors
    ///
    /// Returns [`UserPreferencesError::Io`] if the file cannot be read.
    /// Returns [`UserPreferencesError::Parse`] if the TOML is malformed.
    fn reload(&self) -> Result<UserPreferences, Report<UserPreferencesError>>;

    /// Saves user preferences.
    ///
    /// # Errors
    ///
    /// Returns [`UserPreferencesError::Io`] if writing fails.
    /// Returns [`UserPreferencesError::Parse`] if serialization fails.
    fn save(&self, prefs: &UserPreferences) -> Result<(), Report<UserPreferencesError>>;
}

/// Filesystem-backed user preferences storage.
///
/// Reads from and writes to `jinn.toml` at a configurable path.
/// Production uses `dirs::config_dir()`.
pub struct FilesystemUserPreferencesStorage {
    /// Path to the preferences file.
    path: PathBuf,
}

impl FilesystemUserPreferencesStorage {
    /// Creates a storage backed by an explicit path.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Creates a storage backed by the default config path.
    #[must_use]
    pub fn default_path() -> Self {
        Self {
            path: preferences_path(),
        }
    }

    /// Returns the filesystem path this storage reads from and writes to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl UserPreferencesStorage for FilesystemUserPreferencesStorage {
    fn name(&self) -> &'static str {
        "filesystem"
    }

    fn reload(&self) -> Result<UserPreferences, Report<UserPreferencesError>> {
        super::user_preferences::load_preferences_from(&self.path)
    }

    fn save(&self, prefs: &UserPreferences) -> Result<(), Report<UserPreferencesError>> {
        super::user_preferences::save_preferences_to(prefs, &self.path)
    }
}

/// In-memory user preferences storage for testing.
///
/// Stores the serialized TOML in memory. `reload()` returns the default
/// preferences if nothing has been saved. `save()` stores the serialized
/// preferences.
pub struct InMemoryUserPreferencesStorage {
    /// Serialized TOML content.
    content: Arc<RwLock<Option<String>>>,
}

impl InMemoryUserPreferencesStorage {
    /// Creates an empty in-memory storage (loads default preferences).
    #[must_use]
    pub fn new() -> Self {
        Self {
            content: Arc::new(RwLock::new(None)),
        }
    }
}

impl Default for InMemoryUserPreferencesStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl UserPreferencesStorage for InMemoryUserPreferencesStorage {
    fn name(&self) -> &'static str {
        "in-memory"
    }

    fn reload(&self) -> Result<UserPreferences, Report<UserPreferencesError>> {
        let guard = self.content.read();
        match guard.as_ref() {
            Some(content) => toml::from_str(content)
                .change_context(UserPreferencesError::Parse)
                .attach("failed to parse in-memory preferences"),
            None => Ok(UserPreferences::default()),
        }
    }

    fn save(&self, prefs: &UserPreferences) -> Result<(), Report<UserPreferencesError>> {
        let content = toml::to_string_pretty(prefs)
            .change_context(UserPreferencesError::Parse)
            .attach("failed to serialize preferences")?;
        let mut guard = self.content.write();
        *guard = Some(content);
        Ok(())
    }
}

/// Service wrapper for user preferences storage.
///
/// Wraps `Arc<dyn UserPreferencesStorage>` for shared ownership across the
/// application. Follows the service wrapper pattern from the project style guide.
/// Includes an in-memory cache so repeated reads don't hit disk.
#[derive(Debug, Clone)]
pub struct UserPreferencesStorageService {
    /// The underlying preferences storage implementation.
    svc: Arc<dyn UserPreferencesStorage>,
    /// In-memory cache of the last loaded/saved preferences.
    cache: Arc<RwLock<Option<UserPreferences>>>,
}

impl UserPreferencesStorageService {
    /// Creates a new user preferences storage service.
    #[must_use]
    pub fn new(storage: Arc<dyn UserPreferencesStorage>) -> Self {
        Self {
            svc: storage,
            cache: Arc::new(RwLock::new(None)),
        }
    }

    /// Reads the cached user preferences.
    ///
    /// Infallible. Returns the value cached from the most recent successful
    /// `reload()` or `save()`. **Must** be called after a successful `reload()`
    /// (typically at app startup); panics with a programmer-error message otherwise.
    ///
    /// This method never touches the underlying storage.
    ///
    /// # Panics
    ///
    /// Panics if called before a successful `reload()` (or `save()`) has populated
    /// the cache. The panic message is descriptive — this is a programmer error,
    /// not an expected runtime condition.
    pub fn read(&self) -> UserPreferences {
        self.cache
            .read()
            .clone()
            .expect("UserPreferencesStorageService::read() called before reload() — programmer error: the service must be reloaded (typically at app startup) before any read")
    }

    /// Writes to storage and updates the in-memory cache.
    ///
    /// # Errors
    ///
    /// Returns [`UserPreferencesError::Parse`] if serialization fails.
    pub fn save(&self, prefs: &UserPreferences) -> Result<(), Report<UserPreferencesError>> {
        self.svc.save(prefs)?;
        let mut guard = self.cache.write();
        *guard = Some(prefs.clone());
        Ok(())
    }

    /// Reloads preferences from the underlying storage, bypassing the cache.
    ///
    /// Reads fresh from storage and updates the cache:
    /// - On success, the cache is populated with the result.
    /// - On failure, the cache is **cleared** so that a subsequent `read()` panics
    ///   with the standard "not initialized" message. This prevents the service from
    ///   silently serving stale data after a failed refresh.
    ///
    /// Typically called once at app startup; may be called again later to refresh
    /// the cache from disk.
    ///
    /// # Errors
    ///
    /// Returns [`UserPreferencesError::Io`] if the underlying storage cannot be read.
    /// Returns [`UserPreferencesError::Parse`] if the stored TOML is malformed.
    pub fn reload(&self) -> Result<UserPreferences, Report<UserPreferencesError>> {
        let result = self.svc.reload();
        let mut guard = self.cache.write();
        match &result {
            Ok(prefs) => *guard = Some(prefs.clone()),
            Err(_) => *guard = None,
        }
        result
    }
}

impl std::fmt::Debug for dyn UserPreferencesStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserPreferencesStorage")
            .field("name", &self.name())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::common::app_info::PREFS_FILE_NAME;
    use crate::feat::preferences_actor::RequestRetryConfig;
    use crate::feat::preferences_actor::user_preferences::{
        AutoPruneConfig, BashConfig, CompactionConfig,
        CwdSelectorConfig, MinimapConfig, OpenrouterWebSearchConfig, WebFetchConfig,
    };

    #[rstest::rstest]
    fn in_memory_load_returns_default_when_empty() {
        // Given an empty InMemoryUserPreferencesStorage.
        let storage = InMemoryUserPreferencesStorage::new();

        // When loading.
        let prefs = storage.reload().expect("reload");

        // Then defaults are returned.
        assert!(prefs.last_model.is_none());
    }

    #[rstest::rstest]
    fn in_memory_save_then_load_round_trips() {
        // Given an InMemoryUserPreferencesStorage.
        let storage = InMemoryUserPreferencesStorage::new();
        let prefs = UserPreferences {
            last_model: Some("ollama/llama3".to_owned()),
            last_strategy: None,
            tool_entry_max_lines: None,
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![],
            sidebar_width: None,
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            bash: BashConfig::default(),
        };

        // When saving and reloading.
        storage.save(&prefs).expect("save");
        let reloaded = storage.reload().expect("reload");

        // Then the round-tripped data matches.
        assert_eq!(reloaded.last_model.as_deref(), Some("ollama/llama3"));
    }

    #[rstest::rstest]
    #[rstest::rstest]
    fn filesystem_load_returns_default_when_missing() {
        // Given a temp directory with no file.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let storage = FilesystemUserPreferencesStorage::new(path.clone());

        assert!(!path.exists());

        // When loading.
        let prefs = storage.reload().expect("reload");

        // Then defaults are returned AND the file is auto-created with the
        // canonical template (so users get a comment-rich starter config on
        // first run).
        assert!(prefs.last_model.is_none());
        assert!(
            path.exists(),
            "first-run load should auto-create the config file"
        );
    }

    #[rstest::rstest]
    fn filesystem_save_then_load_round_trips() {
        // Given a FilesystemUserPreferencesStorage in a temp dir.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let storage = FilesystemUserPreferencesStorage::new(path);

        let prefs = UserPreferences {
            last_model: Some("test/model".to_owned()),
            last_strategy: None,
            tool_entry_max_lines: None,
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![],
            sidebar_width: None,
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            bash: BashConfig::default(),
        };

        // When saving and reloading.
        storage.save(&prefs).expect("save");
        let reloaded = storage.reload().expect("reload");

        // Then the round-tripped data matches.
        assert_eq!(reloaded.last_model.as_deref(), Some("test/model"));
    }

    // --- Service caching tests ---

    #[rstest::rstest]
    fn service_load_caches_result() {
        // Given an InMemoryUserPreferencesStorage wrapped in a service.
        let storage = InMemoryUserPreferencesStorage::new();
        let service = UserPreferencesStorageService::new(Arc::new(storage));
        let prefs = UserPreferences {
            last_model: Some("ollama/llama3".to_owned()),
            last_strategy: None,
            tool_entry_max_lines: None,
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![],
            sidebar_width: None,
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            bash: BashConfig::default(),
        };
        service.save(&prefs).expect("save");

        // When loading twice.
        let first = service.reload().expect("first reload");
        let second = service.reload().expect("second reload");

        // Then both return the same value without error.
        assert_eq!(first.last_model, second.last_model);
        assert_eq!(first.last_model.as_deref(), Some("ollama/llama3"));
    }

    #[rstest::rstest]
    fn service_save_updates_cache() {
        // Given a service with cached preferences.
        let storage = InMemoryUserPreferencesStorage::new();
        let service = UserPreferencesStorageService::new(Arc::new(storage));
        let prefs = UserPreferences {
            last_model: Some("ollama/llama3".to_owned()),
            last_strategy: None,
            tool_entry_max_lines: None,
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![],
            sidebar_width: None,
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            bash: BashConfig::default(),
        };
        service.save(&prefs).expect("save");

        // When saving new preferences.
        let updated = UserPreferences {
            last_model: Some("openrouter/gpt-4".to_owned()),
            last_strategy: None,
            tool_entry_max_lines: None,
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![],
            sidebar_width: None,
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            bash: BashConfig::default(),
        };
        service.save(&updated).expect("save updated");

        // Then load returns the updated value.
        let loaded = service.reload().expect("reload");
        assert_eq!(loaded.last_model.as_deref(), Some("openrouter/gpt-4"));
    }

    #[rstest::rstest]
    fn service_reload_clears_cache_and_reads_fresh() {
        // Given a service with cached preferences.
        let storage = InMemoryUserPreferencesStorage::new();
        let service = UserPreferencesStorageService::new(Arc::new(storage));
        let prefs = UserPreferences {
            last_model: Some("ollama/llama3".to_owned()),
            last_strategy: None,
            tool_entry_max_lines: None,
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![],
            sidebar_width: None,
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            bash: BashConfig::default(),
        };
        service.save(&prefs).expect("save");

        // When reloading.
        let reloaded = service.reload().expect("reload");

        // Then fresh preferences are returned from storage.
        assert_eq!(reloaded.last_model.as_deref(), Some("ollama/llama3"));
    }

    #[rstest::rstest]
    #[should_panic(expected = "UserPreferencesStorageService::read() called before reload()")]
    fn read_before_reload_panics_with_precise_message() {
        // Given a service that has never been reloaded or saved.
        let storage = InMemoryUserPreferencesStorage::new();
        let service = UserPreferencesStorageService::new(Arc::new(storage));

        // When reading without prior reload.
        // Then panic with the documented programmer-error message.
        let _ = service.read();
    }

    #[rstest::rstest]
    fn read_after_reload_returns_cached_value() {
        // Given a service reloaded once.
        let storage = InMemoryUserPreferencesStorage::new();
        let service = UserPreferencesStorageService::new(Arc::new(storage));
        let first = service.reload().expect("initial reload");

        // When reading after reload.
        let second = service.read();

        // Then read returns the cached value (equivalent to what reload returned).
        // UserPreferences doesn't impl PartialEq, so compare via Debug repr.
        assert_eq!(format!("{first:?}"), format!("{second:?}"));
    }

    #[rstest::rstest]
    fn reload_failure_returns_err_and_leaves_cache_empty() {
        // Given a service backed by storage that always fails to reload.
        struct AlwaysFails;
        impl UserPreferencesStorage for AlwaysFails {
            fn name(&self) -> &'static str {
                "always-fails"
            }
            fn reload(&self) -> Result<UserPreferences, Report<UserPreferencesError>> {
                Err(Report::new(UserPreferencesError::Parse))
            }
            fn save(&self, _: &UserPreferences) -> Result<(), Report<UserPreferencesError>> {
                Ok(())
            }
        }
        let service = UserPreferencesStorageService::new(Arc::new(AlwaysFails));

        // When reload is called.
        let result = service.reload();

        // Then it returns Err.
        assert!(result.is_err());
    }

    #[rstest::rstest]
    #[should_panic(expected = "read() called before reload")]
    fn read_panics_after_failed_reload() {
        // Given a service backed by storage that always fails to reload.
        struct AlwaysFails;
        impl UserPreferencesStorage for AlwaysFails {
            fn name(&self) -> &'static str {
                "always-fails"
            }
            fn reload(&self) -> Result<UserPreferences, Report<UserPreferencesError>> {
                Err(Report::new(UserPreferencesError::Parse))
            }
            fn save(&self, _: &UserPreferences) -> Result<(), Report<UserPreferencesError>> {
                Ok(())
            }
        }
        let service = UserPreferencesStorageService::new(Arc::new(AlwaysFails));

        // Given reload has failed (cache is empty).
        let _ = service.reload();

        // When read() is called.
        // Then it panics (verified by #[should_panic] above).
        let _ = service.read();
    }
}
