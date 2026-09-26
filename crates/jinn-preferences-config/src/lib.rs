//! `state.toml` schema, the `jinn.toml` default-config bootstrap, and
//! the config section schemas every slice owns.
//!
//! `jinn.toml` itself has no aggregate struct: each section is declared
//! by its owning slice through a [`jinn_config::Configurable`] impl and
//! read through the configuration layer. What remains here is the
//! default-config template used to seed a fresh install.
//!
//! Everything here is *data*: the actor that applies `state.toml` types to
//! `AppState` lives in the `jinn-preferences` slice, and the workers that
//! read the embedded config schemas (prune rules, compaction, retry)
//! keep their behavior in the kernel and import the shapes from here.

#[cfg(test)]
mod template_validation_tests;

pub mod app_state_file;
pub mod app_state_storage;
pub mod config_template;

pub mod protocol;
pub mod schemas;

pub use app_state_file::{AppStateFile, AppStateFileError, load_app_state_from, save_app_state_to};
pub use app_state_storage::{
    AppStateStorage, AppStateStorageService, FilesystemAppStateStorage, InMemoryAppStateStorage,
};
pub use config_template::{
    DEFAULT_CONFIG, InitDefaultConfigError, InitOutcome, UserPreferencesError,
    create_default_preferences_to, init_default_config_to, preferences_path,
};
// The `todo` + `anchored_assistant` auto-prune child configs are schema
// types re-exported so the kernel's workers and every consumer keep one
// import home for the `[context_curation.auto_prune]` section.
pub use schemas::auto_prune::{AnchoredAssistantAutoPruneConfig, TodoAutoPruneConfig};
// Watchdog sections: consumed by the `jinn-watchdog` slice at activation.
pub use schemas::StallWatchdogConfig;
pub use schemas::ToolCallWatchdogConfig;

// The configuration layer lives in its own kernel-free crate so
// `jinn-slices` can hold one without a dependency cycle. Re-exported here
// because this is where a consumer already looks for configuration.
pub use jinn_config::ConfigLayer;
