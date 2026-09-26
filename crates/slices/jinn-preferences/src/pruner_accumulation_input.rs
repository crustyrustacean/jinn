//! The pruner accumulation threshold input popup.
//!
//! Opened from normal mode with `gcp`, the popup edits a numeric token
//! threshold in its own preferences-owned cell. Confirmation persists the new
//! threshold through the configuration layer's `put`.

pub mod intent;
pub mod render;
pub mod state;
