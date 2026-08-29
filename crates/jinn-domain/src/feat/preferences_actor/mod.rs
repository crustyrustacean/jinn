//! User preferences persistence - manages `~/.config/jinn/jinn.toml`.
//!
//! Provides the [`UserPreferences`] data type, storage trait, and service wrapper
//! for reading and writing user preferences across app restarts. The preferences
//! actor subscribes to provider switch events and persists the last-used model.

pub mod app_state_actor;
pub mod app_state_file;
pub mod app_state_storage;
#[expect(
    clippy::module_inception,
    reason = "preferences_actor/mod.rs is the public API, preferences_actor/ is implementation"
)]
pub mod preferences_actor;
pub mod protocol;
pub mod user_preferences;
pub mod user_preferences_storage;
pub use app_state_storage::{
    AppStateStorageService, FilesystemAppStateStorage, InMemoryAppStateStorage,
};
pub use user_preferences::{
    AutoPruneConfig, CompactionConfig, InitDefaultConfigError, InitOutcome, MinimapConfig,
    OpenrouterWebSearchConfig, RequestRetryConfig, TaskListPreferences, UserPreferences,
    init_default_config_to, preferences_path,
};
pub use user_preferences_storage::{
    FilesystemUserPreferencesStorage, InMemoryUserPreferencesStorage, UserPreferencesStorageService,
};
