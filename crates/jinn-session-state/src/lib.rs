//! Authoritative live session state and snapshot contracts.
//!
//! This neutral crate owns the coherent session aggregate that turn, store,
//! lifecycle, and frontend adapters coordinate through. It does not depend on
//! those implementation slices or on the domain kernel.

#![forbid(unsafe_code)]

pub mod assembly_projection;
pub mod chat_session;
pub mod core;
pub mod fields;
pub mod mutation_accumulator;
pub mod read_projection;
mod runtime;
pub mod session_map;
pub mod snapshot;
pub mod steering_buffer;
pub mod turn_count;
mod tree_projection;

pub use assembly_projection::AssemblySessionProjection;
pub use chat_session::{ChatSessionState, StreamingError};
pub use core::SessionCore;
pub use fields::{
    SessionHistoryWorkFields, SessionIdentityMetadataFields, SessionIntegrationFields,
    SessionLifecycleLocationFields, SessionProfile, SessionStorageFields, default_cwd,
    default_persist,
};
pub use read_projection::SessionReadProjection;
pub use runtime::{SessionCoreEphemeral, SessionUi};
pub use session_map::{SessionLoadGuard, SessionMap};
pub use snapshot::{SessionRevision, SessionSnapshot, SessionSnapshotMetadata};
pub use turn_count::compute_turn_count;
pub use tree_projection::{snapshot_frozen_node, snapshot_frozen_node_from_snapshot};
