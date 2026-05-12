//! User preferences persistence — manages `~/.config/nullslop/nullslop.toml`.
//!
//! Provides the [`UserPreferences`] data type, storage trait, and service wrapper
//! for reading and writing user preferences across app restarts. The preferences
//! actor subscribes to provider switch events and persists the last-used model.

pub mod preferences_actor;
pub mod user_preferences;
pub mod user_preferences_storage;

pub use user_preferences::UserPreferences;
pub use user_preferences_storage::{
    FilesystemUserPreferencesStorage, InMemoryUserPreferencesStorage,
    UserPreferencesStorageService,
};
pub use preferences_actor::spawn_preferences_actor;
