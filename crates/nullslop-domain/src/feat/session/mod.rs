//! Session management — session lifecycle, persistence, and loading.
//!
//! Provides persistence types ([`PersistedSession`], [`SessionStore`], etc.)
//! used by the session actor, services container, and component crate.
//! Also contains the session actor, intent handlers, validators, entry loaders,
//! and picker rendering.

mod persisted_session;
pub mod session_store;

pub mod chat_entry;
pub mod chat_session;
pub mod entries;
pub mod intent;
pub mod picker_entry;
pub mod protocol;
pub mod render;
pub mod session_actor;
pub mod token_stats;
pub mod validator;

pub use chat_session::{ChatSessionState, SessionCore, SessionUi};

pub use persisted_session::{BLOB_STRATEGY_STATE, PersistedSession, SessionSummary};
pub use session_store::{JsonlSessionStore, SessionStore, SessionStoreError, SessionStoreService};
pub use token_stats::{
    AggregatedTokenStats, BLOB_PARENT_SESSION, BLOB_TOKEN_STATS, TokenRecord, TokenStats,
    aggregate_session_stats,
};
