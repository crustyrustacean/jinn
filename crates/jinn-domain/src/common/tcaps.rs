//! TCaps: Token-based Capability System.
//!
//! Write access to shared state is gated by unforgeable ZST tokens (caps).
//! Each cap projects a narrow borrowed view. Reads remain a full snapshot.
//!
//! This module is the root for the `tcaps/` subtree. Caps, views, and traits
//! live one-per-file under `tcaps/`; `mint.rs` is the single trust point where
//! caps are constructed.

pub mod mint;
