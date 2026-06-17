//! Re-export of [`SessionRegistryId`] from the parent module.
//!
//! Historical artifact: jinn-plugin had this module so `use crate::session_registry`
//! would resolve. After merge, this just re-exports from the parent.

pub use super::SessionRegistryId;
