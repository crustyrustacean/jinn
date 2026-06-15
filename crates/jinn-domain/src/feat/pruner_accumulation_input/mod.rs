//! Pruner accumulation threshold input popup — adjust the token threshold.
//!
//! Provides a numeric-only text input popup that seeds with the current
//! pruner-accumulation threshold (from `jinn.toml`), accepts digits only, and
//! on confirm persists the new value via `UpdatePreferences` so the
//! `PreferencesActor` saves it to `jinn.toml` and broadcasts the change.

pub mod intent;
pub mod render;
pub mod state;
