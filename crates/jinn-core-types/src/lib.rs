//! Foundational, domain-agnostic value types shared across the jinn workspace.
//!
//! Residents here are pure value types (newtypes over primitives) with no
//! dependency on domain logic, actors, or app state. They exist so that leaf
//! crates — e.g. a future plugin engine — can reference a shared type without
//! depending on `jinn-domain`.
//!
//! Types are added as-needed. This is not a dumping ground: only types that are
//! both foundational and domain-agnostic belong here.

pub mod attached_plugin;
pub mod plugin_instance_id;
pub mod session_id;
pub mod session_registry_id;

pub use attached_plugin::{AttachedPlugin, PluginRunState};
pub use plugin_instance_id::PluginInstanceId;
pub use session_id::SessionId;
pub use session_registry_id::SessionRegistryId;
