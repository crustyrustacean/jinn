//! The pruner accumulation threshold input popup.
//!
//! Opened from normal mode with `gcp`, the popup edits a numeric token
//! threshold in its own preferences-owned cell. Confirmation emits
//! `UpdatePreferences { SetAccumulationThreshold }`; persistence remains the
//! preferences actor's responsibility.

pub mod intent;
pub mod render;
pub mod state;
