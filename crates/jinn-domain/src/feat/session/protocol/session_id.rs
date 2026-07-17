//! Re-export of [`SessionId`] from the `jinn-core-types` crate.
//!
//! The definition moved to `jinn-core-types` (a domain-agnostic leaf crate) so
//! that crates outside `jinn-domain` can reference it without a back-dependency. This
//! shim keeps the historical path
//! `crate::feat::session::protocol::session_id::SessionId` (and the
//! `protocol::SessionId` / root `SessionId` re-exports) resolving for the 140+
//! in-crate consumers.
pub use jinn_core_types::SessionId;
