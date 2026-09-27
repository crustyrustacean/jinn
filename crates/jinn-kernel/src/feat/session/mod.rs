//! Session management - session lifecycle, persistence, and loading.
//!
//! Contains the session intent handlers and their validators. The
//! persistence seam ([`SessionStore`], [`SessionStoreService`]) lives in
//! `jinn_session_state` beside `SessionSnapshot`, so the kernel's service
//! container no longer reaches into a feature module for its storage type.
//! Picker entry loading lives in the `jinn-session-store` slice, which owns
//! the picker.

pub mod intent;

pub use jinn_core_types::SessionProfile;
pub use jinn_session_state::{
    FrozenTreeNode, SessionStore, SessionStoreError, SessionStoreService, aggregate_session_stats,
    aggregate_tree_stats, find_tree_root, snapshot_frozen_node, snapshot_frozen_node_from_snapshot,
};
