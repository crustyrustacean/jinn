//! Plugin dispatch — thin event → hook dispatcher.
//!
//! Replaces `WorkflowControllerActor`. Listens to lifecycle events and
//! translates them into plugin hook fires.

use std::fmt;

/// Where a plugin hook failed — attached to plugin-error `Report`s so that
/// `tracing::error!(error = ?e)` renders the plugin name + hook name in the
/// chain. error-stack renders `.attach()` (printable) values via `Display`.
#[derive(Debug)]
pub struct PluginHookSite {
    pub plugin: String,
    pub hook: String,
}

impl fmt::Display for PluginHookSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "plugin hook {}.{}", self.plugin, self.hook)
    }
}

pub mod actor;
pub mod domain_node_context;
pub mod hook_context;
pub mod picker_entry;
pub mod plugin_fire;
pub mod plugin_sync;
pub mod plugin_sync_hooks;
pub mod protocol;
pub mod session_plugin_registry;
pub mod session_plugin_registry_service;

pub use actor::{PluginDispatchActor, PluginDispatchActorDeps};
pub use domain_node_context::DomainNodeContext;
pub use hook_context::{HookContext, ProvidesSessionId};
pub use picker_entry::PluginPickerEntry;
pub use plugin_fire::{PluginFire, PluginFireError, PluginFireService};
pub use plugin_sync::{PluginSyncCall, PluginSyncCallError, PluginSyncCallService};
pub use plugin_sync_hooks::{
    BadgeDirective, BadgeSegment, InterceptOutcome, PluginSyncHooks, PreviewDirective,
    call_hooks_typed,
};
pub use session_plugin_registry::{
    CreateSessionRegistryResult, PluginToolMetadata, SessionPluginRegistry,
    SessionPluginRegistryError, ToolScope,
};
pub use session_plugin_registry_service::SessionPluginRegistryService;
