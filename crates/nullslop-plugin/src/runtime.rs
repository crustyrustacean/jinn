//! Per-plugin rhai runtime.
//!
//! Each plugin gets its own [`PluginRuntime`] wrapping a `rhai::Engine`,
//! `rhai::Scope`, and compiled `AST`. The runtime tracks whether the plugin
//! is enabled and provides methods to call the plugin's `init()` and
//! `on_event()` functions.

use std::path::Path;
use std::sync::Arc;

use rhai::{AST, Engine, Map, Scope};

use crate::PluginError;
use crate::plugin_id::PluginId;

/// Callbacks that bridge rhai host API calls to Rust state.
///
/// Created by the PluginActor's plugin thread and injected into each PluginRuntime.
pub struct HostCallbacks {
    /// Subscribe to event types.
    pub subscribe_events: Arc<dyn Fn(&PluginId, &[String]) + Send + Sync>,
    /// Emit a custom inter-plugin event.
    #[allow(dead_code)]
    pub emit_event: Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>,
    /// Add or update a status bar slot.
    pub upsert_slot: Arc<dyn Fn(PluginSlotInfo) + Send + Sync>,
    /// Update an existing slot's text.
    pub update_slot: Arc<dyn Fn(&PluginId, &str, &str) + Send + Sync>,
    /// Get chat entries for the active session.
    pub get_entries: Arc<dyn Fn() -> Vec<Map> + Send + Sync>,
    /// Send an allowlisted command.
    #[allow(dead_code)]
    pub send_command: Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>,
}

impl std::fmt::Debug for HostCallbacks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostCallbacks").finish_non_exhaustive()
    }
}

/// Info about a status bar slot, passed from rhai to Rust.
#[derive(Debug, Clone)]
pub struct PluginSlotInfo {
    /// Which plugin owns this slot.
    pub plugin_id: PluginId,
    /// Stable identifier provided by the plugin.
    pub stable_id: String,
    /// Which side of the status bar ("left" or "right").
    pub section: String,
    /// Ordering within section (lower = first).
    pub priority: i64,
    /// The current text to display.
    pub text: String,
}

/// A single loaded plugin with its own rhai engine.
pub struct PluginRuntime {
    /// The plugin's identity.
    pub plugin_id: PluginId,
    /// Whether the plugin is currently active.
    pub enabled: bool,
    engine: Engine,
    scope: Scope<'static>,
    ast: AST,
}

impl std::fmt::Debug for PluginRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginRuntime")
            .field("plugin_id", &self.plugin_id)
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

impl PluginRuntime {
    /// Loads a plugin from the given `main.rhai` file path.
    ///
    /// Creates a fresh rhai engine, registers host API functions,
    /// compiles the script, and evaluates it to set up top-level state.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::LoadFailed`] if the file cannot be read.
    /// Returns [`PluginError::EvalFailed`] if the script has syntax errors.
    pub fn load(
        plugin_id: PluginId,
        path: &Path,
        callbacks: Arc<HostCallbacks>,
    ) -> Result<Self, PluginError> {
        let source = std::fs::read_to_string(path).map_err(|_| PluginError::LoadFailed)?;

        let mut engine = Engine::new();

        // --- Register host API functions as global functions ---

        let cb = callbacks.clone();
        let pid = plugin_id.clone();
        engine.register_fn("host_events_subscribe", move |types: rhai::Array| {
            let names: Vec<String> = types
                .into_iter()
                .filter_map(|v| v.into_string().ok())
                .collect();
            (cb.subscribe_events)(&pid, &names);
        });

        let cb = callbacks.clone();
        let pid = plugin_id.clone();
        engine.register_fn(
            "host_status_bar_add_slot",
            move |id: &str, section: &str, priority: i64, text: &str| {
                let info = PluginSlotInfo {
                    plugin_id: pid.clone(),
                    stable_id: id.to_owned(),
                    section: section.to_owned(),
                    priority,
                    text: text.to_owned(),
                };
                (cb.upsert_slot)(info);
                id.to_owned()
            },
        );

        let cb = callbacks.clone();
        let pid = plugin_id.clone();
        engine.register_fn(
            "host_status_bar_update_slot",
            move |id: &str, text: &str| {
                (cb.update_slot)(&pid, id, text);
            },
        );

        let cb = callbacks.clone();
        engine.register_fn("host_chat_get_entries", move || -> rhai::Array {
            (cb.get_entries)()
                .into_iter()
                .map(rhai::Dynamic::from)
                .collect()
        });

        // Compile and evaluate the script.
        let ast = engine.compile(&source).map_err(|e| {
            tracing::error!(plugin = %plugin_id, err = ?e, "script compilation failed");
            PluginError::EvalFailed
        })?;

        let mut scope = Scope::new();
        engine.run_ast_with_scope(&mut scope, &ast).map_err(|e| {
            tracing::error!(plugin = %plugin_id, err = ?e, "script evaluation failed");
            PluginError::EvalFailed
        })?;

        Ok(Self {
            plugin_id,
            enabled: true,
            engine,
            scope,
            ast,
        })
    }

    /// Calls the plugin's `init()` function.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::InitFailed`] if the function throws.
    /// Returns [`PluginError::Disabled`] if the plugin is disabled.
    pub fn call_init(&mut self) -> Result<(), PluginError> {
        if !self.enabled {
            return Err(PluginError::Disabled);
        }

        self.engine
            .call_fn::<()>(&mut self.scope, &self.ast, "init", ())
            .map_err(|e| {
                tracing::error!(plugin = %self.plugin_id, err = ?e, "init() threw");
                self.enabled = false;
                PluginError::InitFailed
            })
    }

    /// Calls the plugin's `on_event(event)` function.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::OnEventFailed`] if the function throws.
    /// Returns [`PluginError::Disabled`] if the plugin is disabled.
    pub fn call_on_event(&mut self, event: Map) -> Result<(), PluginError> {
        if !self.enabled {
            return Err(PluginError::Disabled);
        }

        self.engine
            .call_fn::<()>(&mut self.scope, &self.ast, "on_event", (event,))
            .map_err(|e| {
                tracing::error!(plugin = %self.plugin_id, err = ?e, "on_event() threw");
                self.enabled = false;
                PluginError::OnEventFailed
            })
    }

    /// Disables the plugin.
    pub fn disable(&mut self) {
        self.enabled = false;
    }
}
