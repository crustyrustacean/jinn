//! Plugin dispatch — thin event → hook dispatcher.
//!
//! Replaces `WorkflowControllerActor`. Listens to lifecycle events and
//! translates them into plugin hook fires.

pub mod actor;
pub mod domain_node_context;
pub mod picker_entry;
pub mod plugin_fire;
pub mod plugin_sync;
pub mod protocol;

pub use actor::{PluginDispatchActor, PluginDispatchActorDeps};
pub use domain_node_context::DomainNodeContext;
pub use picker_entry::PluginPickerEntry;
pub use plugin_fire::{PluginFire, PluginFireError, PluginFireService};
pub use plugin_sync::{PluginSyncCall, PluginSyncCallError, PluginSyncCallService};
