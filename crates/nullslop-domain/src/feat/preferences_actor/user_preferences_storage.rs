//! User preferences storage abstraction — trait for user preferences I/O.
//!
//! Defines [`UserPreferencesStorage`] as the abstraction for loading and saving
//! user preferences. [`FilesystemUserPreferencesStorage`] is the production
//! implementation; [`InMemoryUserPreferencesStorage`] is for testing.

use std::path::PathBuf;
use std::sync::Arc;

use error_stack::{Report, ResultExt as _};
use parking_lot::RwLock;

use super::user_preferences::{UserPreferences, UserPreferencesError, preferences_path};

/// Trait for user preferences I/O.
///
/// Every external dependency must have a trait abstraction.
/// Filesystem I/O is an external dependency — this trait abstracts it so
/// tests can use in-memory storage instead of touching the real filesystem.
pub trait UserPreferencesStorage: Send + Sync + 'static {
    /// Returns the storage backend name (for debugging).
    fn name(&self) -> &'static str;

    /// Loads user preferences.
    ///
    /// Returns default preferences if none exist.
    ///
    /// # Errors
    ///
    /// Returns [`UserPreferencesError::Io`] if the file cannot be read.
    /// Returns [`UserPreferencesError::Parse`] if the TOML is malformed.
    fn load(&self) -> Result<UserPreferences, Report<UserPreferencesError>>;

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
/// Reads from and writes to `nullslop.toml` at a configurable path.
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
}

impl UserPreferencesStorage for FilesystemUserPreferencesStorage {
    fn name(&self) -> &'static str {
        "filesystem"
    }

    fn load(&self) -> Result<UserPreferences, Report<UserPreferencesError>> {
        super::user_preferences::load_preferences_from(&self.path)
    }

    fn save(&self, prefs: &UserPreferences) -> Result<(), Report<UserPreferencesError>> {
        super::user_preferences::save_preferences_to(prefs, &self.path)
    }
}

/// In-memory user preferences storage for testing.
///
/// Stores the serialized TOML in memory. `load()` returns the default
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

    fn load(&self) -> Result<UserPreferences, Report<UserPreferencesError>> {
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
#[derive(Debug, Clone)]
pub struct UserPreferencesStorageService {
    /// The underlying preferences storage implementation.
    svc: Arc<dyn UserPreferencesStorage>,
}

impl UserPreferencesStorageService {
    /// Creates a new user preferences storage service.
    #[must_use]
    pub fn new(storage: Arc<dyn UserPreferencesStorage>) -> Self {
        Self { svc: storage }
    }

    /// Loads user preferences.
    ///
    /// # Errors
    ///
    /// Returns [`UserPreferencesError::Parse`] if stored content is malformed.
    pub fn load(&self) -> Result<UserPreferences, Report<UserPreferencesError>> {
        self.svc.load()
    }

    /// Saves user preferences.
    ///
    /// # Errors
    ///
    /// Returns [`UserPreferencesError::Parse`] if serialization fails.
    pub fn save(&self, prefs: &UserPreferences) -> Result<(), Report<UserPreferencesError>> {
        self.svc.save(prefs)
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
    use super::*;

    #[rstest::rstest]
    fn in_memory_load_returns_default_when_empty() {
        // Given an empty InMemoryUserPreferencesStorage.
        let storage = InMemoryUserPreferencesStorage::new();

        // When loading.
        let prefs = storage.load().expect("load");

        // Then defaults are returned.
        assert!(prefs.last_model.is_none());
    }

    #[rstest::rstest]
    fn in_memory_save_then_load_round_trips() {
        // Given an InMemoryUserPreferencesStorage.
        let storage = InMemoryUserPreferencesStorage::new();
        let prefs = UserPreferences {
            last_model: Some("ollama/llama3".to_owned()),
        };

        // When saving and reloading.
        storage.save(&prefs).expect("save");
        let reloaded = storage.load().expect("load");

        // Then the round-tripped data matches.
        assert_eq!(reloaded.last_model.as_deref(), Some("ollama/llama3"));
    }

    #[rstest::rstest]
    fn filesystem_load_returns_default_when_missing() {
        // Given a temp directory with no file.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("nullslop.toml");
        let storage = FilesystemUserPreferencesStorage::new(path.clone());

        assert!(!path.exists());

        // When loading.
        let prefs = storage.load().expect("load");

        // Then defaults are returned (file is NOT auto-created on load).
        assert!(prefs.last_model.is_none());
        assert!(!path.exists());
    }

    #[rstest::rstest]
    fn filesystem_save_then_load_round_trips() {
        // Given a FilesystemUserPreferencesStorage in a temp dir.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("nullslop.toml");
        let storage = FilesystemUserPreferencesStorage::new(path);

        let prefs = UserPreferences {
            last_model: Some("test/model".to_owned()),
        };

        // When saving and reloading.
        storage.save(&prefs).expect("save");
        let reloaded = storage.load().expect("load");

        // Then the round-tripped data matches.
        assert_eq!(reloaded.last_model.as_deref(), Some("test/model"));
    }
}
