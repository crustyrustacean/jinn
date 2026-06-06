//! User preferences persistence - manages `~/.config/jinn/jinn.toml`.
//!
//! Provides the [`UserPreferences`] data type, storage trait, and service wrapper
//! for reading and writing user preferences across app restarts. The preferences
//! actor subscribes to provider switch events and persists the last-used model.

pub mod preferences_actor;
pub mod preferences_state_sync_actor;
pub mod protocol;
pub mod user_preferences;
pub mod user_preferences_storage;

#[cfg(test)]
mod preferences_actor_tests;

pub use user_preferences::{
    AutoPruneConfig, BashConfig, CompactionConfig, ContextSlidingWindowConfig,
    InitDefaultConfigError, InitOutcome, MinimapConfig, OpenrouterWebSearchConfig,
    RequestRetryConfig, UserPreferences, init_default_config_to, preferences_path,
};
pub use user_preferences_storage::{
    FilesystemUserPreferencesStorage, InMemoryUserPreferencesStorage, UserPreferencesStorageService,
};
