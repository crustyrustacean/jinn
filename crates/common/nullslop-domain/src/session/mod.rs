//! Session management — session lifecycle, persistence, and loading.
//!
//! Provides persistence types ([`PersistedSession`], [`SessionStore`], etc.)
//! used by the session actor, services container, and component crate.

mod persisted_session;
pub mod session_store;

pub use persisted_session::{BLOB_STRATEGY_STATE, PersistedSession, SessionSummary};
pub use session_store::{JsonlSessionStore, SessionStore, SessionStoreError, SessionStoreService};
