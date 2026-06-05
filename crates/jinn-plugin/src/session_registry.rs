//! Re-export of [`SessionRegistryId`] from `jinn-domain`.
//!
//! Kept as a module so `use crate::session_registry::SessionRegistryId`
//! resolves in places like `async_handle.rs`.

pub use jinn_domain::feat::plugin_system::SessionRegistryId;
