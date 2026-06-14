//! Session plugin registry trait — abstraction over per-session Lua state lifecycle.
//!
//! `jinn-domain` cannot depend on `jinn-plugin` (circular dependency), so this
//! trait provides the minimal interface for the `PluginDispatchActor` to manage
//! per-session Lua states via the `Services` DI container.

use error_stack::Report;
use wherror::Error;

use crate::feat::plugin_system::SessionRegistryId;
use crate::protocol::SessionId;

/// Result of creating a per-session plugin registry.
#[derive(Debug)]
pub struct CreateSessionRegistryResult {
    /// The newly created registry ID.
    pub registry_id: SessionRegistryId,
    /// Tool definitions extracted from the loaded plugins.
    /// Each item carries plugin name + tool definition for registration
    /// with the tools actor.
    pub tool_metadata: Vec<PluginToolMetadata>,
}

/// Send-safe metadata about a plugin-defined tool.
#[derive(Debug, Clone)]
pub struct PluginToolMetadata {
    /// Tool name (e.g., "judgment_passed").
    pub name: String,
    /// Human-readable description for the LLM.
    pub description: String,
    /// Full JSON Schema for parameters.
    pub parameters: serde_json::Value,
    /// Plugin that defines this tool.
    pub plugin_name: String,
    /// Whether this tool is global or session-attached.
    pub scope: ToolScope,
}

impl PluginToolMetadata {
    /// Convert this metadata into a [`ToolDefinition`] for registration.
    pub fn to_tool_definition(&self) -> jinn_provider::ToolDefinition {
        jinn_provider::ToolDefinition {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
            prompt_snippet: None,
            prompt_guidelines: Vec::new(),
            server_tool_type: None,
        }
    }
}

/// Whether a plugin tool is available globally or only in the session it was attached to.
///
/// Set via the `scope` field in Lua tool definitions: `scope = "global"` or `scope = "attached"`.
/// Defaults to `Attached` when omitted (least privilege).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolScope {
    /// Available in all sessions.
    Global,
    /// Available only in the session the plugin is attached to.
    #[default]
    Attached,
}

/// Error raised by [`SessionPluginRegistry`] implementations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct SessionPluginRegistryError;

/// Manage per-session plugin Lua states.
///
/// Implemented by `jinn_plugin::AsyncPluginHandle`. The dispatcher uses this
/// trait to spin up isolated Lua states for each session's attached plugins
/// and tear them down on detach.
#[async_trait::async_trait]
pub trait SessionPluginRegistry: Send + Sync {
    /// Create a per-session Lua state with the named attachable plugins loaded.
    ///
    /// Returns an opaque [`SessionRegistryId`] used in subsequent
    /// `PluginFire::fire_async_for_session_json` calls.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is unreachable or any named plugin
    /// cannot be loaded.
    async fn create_session_registry(
        &self,
        plugin_names: Vec<String>,
        origin_session_id: SessionId,
    ) -> Result<CreateSessionRegistryResult, Report<SessionPluginRegistryError>>;

    /// Drop a per-session Lua state.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is dead.
    async fn destroy_session_registry(
        &self,
        registry_id: SessionRegistryId,
    ) -> Result<(), Report<SessionPluginRegistryError>>;

    /// Returns the name of this backend for debugging.
    fn name(&self) -> &'static str;
}
