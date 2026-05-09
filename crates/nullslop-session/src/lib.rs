//! Session persistence — serializable session snapshots and lazy-load summaries.
//!
//! Defines [`PersistedSession`] for durable session data and [`SessionSummary`]
//! for lightweight startup scanning. Subsystem state (strategies) is stored as
//! opaque blobs keyed by well-known constants.
//!
//! [`SessionStore`] abstracts the storage backend; [`JsonlSessionStore`] is the
//! append-only JSONL file implementation. [`SessionStoreService`] wraps the
//! trait for shared ownership across the application.

mod persisted_session;
pub mod session_store;

pub use persisted_session::{BLOB_STRATEGY_STATE, PersistedSession, SessionSummary};
pub use session_store::{JsonlSessionStore, SessionStore, SessionStoreError, SessionStoreService};
