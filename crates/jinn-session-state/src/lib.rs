//! Authoritative live session state and snapshot contracts.
//!
//! This neutral crate owns the coherent session aggregate that turn, store,
//! lifecycle, and frontend adapters coordinate through. It does not depend on
//! those implementation slices or on the domain kernel.

#![forbid(unsafe_code)]

pub mod chat_session;
pub mod core;
pub mod fields;
pub mod mutation_accumulator;
mod runtime;
pub mod session_map;
pub mod steering_buffer;

pub use chat_session::{ChatSessionState, StreamingError};
pub use core::SessionCore;
pub use fields::{
    SessionHistoryWorkFields, SessionIdentityMetadataFields, SessionIntegrationFields,
    SessionLifecycleLocationFields, SessionProfile, SessionStorageFields, default_cwd,
    default_persist,
};
pub use runtime::{SessionCoreEphemeral, SessionUi};
pub use session_map::{SessionLoadGuard, SessionMap};
