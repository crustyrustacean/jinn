//! Session management — session lifecycle, persistence, and loading.
//!
//! Provides persistence types ([`ChatSessionState`], [`SessionStore`], etc.)
//! used by the session actor, services container, and component crate.
//! Also contains the session actor, intent handlers, validators, entry loaders,
//! and picker rendering.

pub mod session_store;
pub mod session_summary;

pub mod chat_entry;
pub mod chat_session;
pub mod entries;
pub mod intent;
pub mod picker_entry;
pub mod profile;
pub mod protocol;
pub mod render;
pub mod session_actor;
pub mod token_stats;
pub mod validator;

pub use chat_session::{ChatSessionState, SessionCore, SessionUi};
pub use profile::SessionProfile;
pub use session_store::{JsonlSessionStore, SessionStore, SessionStoreError, SessionStoreService};
pub use session_summary::SessionSummary;
pub use token_stats::{AggregatedTokenStats, TokenRecord, TokenStats, aggregate_session_stats};
