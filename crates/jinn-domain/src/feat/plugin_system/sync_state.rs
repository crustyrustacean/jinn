#![expect(dead_code, reason = "sync hook caching not yet wired")]
//! Sync-side plugin handle — owns the Lua state that runs on the render thread.
//!
//! [`SyncPlugins`] is `!Send` because `mlua::Lua` is `!Send`. It must live
//! on the render thread. Sync hooks are called directly, with no thread hops.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;

use error_stack::{Report, ResultExt};
use mlua::{Lua, LuaSerdeExt, RegistryKey, Value};
use serde::de::DeserializeOwned;
use wherror::Error;

use super::bindings;
use super::command::PluginCommand;
use super::plugin_data::PluginData;
use crate::SessionId;
use crate::feat::plugin_dispatch::{HookContext, PluginHookSite};

/// Stored hook data for a loaded plugin.
pub struct PluginHooks {
    /// Registry key for the plugin's returned table.
    table: RegistryKey,
    /// Cache of hook names known to exist on this plugin's table.
    hook_cache: RefCell<HashSet<String>>,
}

impl PluginHooks {
    /// Create a new `PluginHooks` from its registry key.
    pub(crate) fn new(table: RegistryKey) -> Self {
        Self {
            table,
            hook_cache: RefCell::new(HashSet::new()),
        }
    }

    /// Registry key for the plugin's returned table.
    pub(crate) fn table(&self) -> &RegistryKey {
        &self.table
    }

    /// Mutable cache of hook names known to exist on this plugin's table.
    pub(crate) fn hook_cache(&self) -> &RefCell<HashSet<String>> {
        &self.hook_cache
    }
}

/// A keybind declared by a plugin, deserialized from the plugin module's
/// `keybinds` table. Consumed by the TUI to register dynamic bindings.
#[derive(Debug, Clone)]
pub struct PluginKeybind {
    /// Name of the plugin that declared this keybind.
    pub plugin_name: String,
    /// The key sequence (e.g. `"<M-e>"`).
    pub keys: String,
    /// The action name the plugin registered.
    pub action: String,
    /// Human-readable description shown in which-key help.
    pub description: String,
    /// Target scope (e.g. `"input"`, `"normal"`), parsed by [`Scope::from_str`].
    pub scope: String,
}

/// Intermediate serde shape for a single entry of the plugin's `keybinds`
/// table. Maps the Lua field names to the strongly typed [`PluginKeybind`].
#[derive(Debug, Clone, serde::Deserialize)]
struct PluginKeybindRaw {
    /// The key sequence string (e.g. `"<M-e>"`).
    keys: String,
    /// The action hook name to fire on the async VM when the keybind triggers.
    action: String,
    /// Human-readable description shown in the which-key help popup.
    description: String,
    /// Target keymap scope (e.g. `"input"`, `"normal"`).
    scope: String,
}

impl PluginKeybindRaw {
    /// Convert the raw serde shape into a typed keybind, attaching the
    /// declaring plugin's name.
    fn into_keybind(self, plugin_name: String) -> PluginKeybind {
        PluginKeybind {
            plugin_name,
            keys: self.keys,
            action: self.action,
            description: self.description,
            scope: self.scope,
        }
    }
}

/// Error type for failures in the sync plugin path (render thread).
///
/// Colocated with [`SyncHook`] because it is the primary consumer of this
/// error. Specific failure reasons are attached to the `Report` via
/// `.attach("...")` calls.
#[derive(Debug, Error)]
#[error(debug)]
pub struct PluginSyncStateError;

/// Owns the sync Lua state. `!Send` — must live on the render thread.
pub struct SyncPlugins {
    /// The Lua VM owning all sync plugin state.
    lua: Lua,
    /// Plugin name → hook data.
    hooks: HashMap<String, PluginHooks>,
    /// Shared plugin data store.
    plugin_data: PluginData,
    /// Channel for emitting commands from sync hooks.
    emit_tx: kanal::Sender<PluginCommand>,
    /// Shared in-flight-request registry. Lets sync hooks cancel
    /// async `ctx.request`s via `ctx.cancel(task)` (see Phase 2).
    in_flight: super::InFlightRequests,
}

impl SyncPlugins {
    /// Construct a `SyncPlugins` from its owned parts.
    ///
    /// Called by [`crate::PluginSystem::build`] to assemble the render-thread
    /// plugin state from the loaded Lua VM, hook map, and shared channels.
    pub(crate) fn new(
        lua: Lua,
        hooks: HashMap<String, PluginHooks>,
        plugin_data: PluginData,
        emit_tx: kanal::Sender<PluginCommand>,
        in_flight: super::InFlightRequests,
    ) -> Self {
        Self {
            lua,
            hooks,
            plugin_data,
            emit_tx,
            in_flight,
        }
    }

    /// Shared plugin data store (used to clone into the async handle).
    pub(crate) fn plugin_data(&self) -> &PluginData {
        &self.plugin_data
    }
}

impl Default for SyncPlugins {
    fn default() -> Self {
        let (emit_tx, _) = kanal::unbounded::<PluginCommand>();
        Self::new(
            Lua::new(),
            HashMap::new(),
            PluginData::new(),
            emit_tx,
            super::InFlightRequests::new(),
        )
    }
}

/// A single sync hook ready to be called.
pub struct SyncHook<'a> {
    /// The Lua VM.
    lua: &'a Lua,
    /// Name of the plugin this hook belongs to.
    plugin_name: String,
    /// The hook function to call.
    func: mlua::Function,
    /// Shared plugin data store.
    plugin_data: &'a PluginData,
    emit_tx: kanal::Sender<PluginCommand>,
    /// Shared in-flight-request registry for ctx.cancel().
    in_flight: &'a super::InFlightRequests,
}

impl<'a> SyncHook<'a> {
    /// Construct a single sync hook ready to be called.
    pub(crate) fn new(
        lua: &'a Lua,
        plugin_name: String,
        func: mlua::Function,
        plugin_data: &'a PluginData,
        emit_tx: kanal::Sender<PluginCommand>,
        in_flight: &'a super::InFlightRequests,
    ) -> Self {
        Self {
            lua,
            plugin_name,
            func,
            plugin_data,
            emit_tx,
            in_flight,
        }
    }
}

impl SyncHook<'_> {
    /// The name of the plugin that owns this hook.
    pub(crate) fn plugin_name(&self) -> &str {
        &self.plugin_name
    }
    /// Call this hook with context data and deserialize the return value.
    ///
    /// # Type Parameters
    ///
    ///
    /// - `T` — the context struct (must be `Serialize`)
    /// - `R` — the expected return type (must be `DeserializeOwned`)
    ///
    /// # Errors
    ///
    /// Returns `Err` if serialization, Lua execution, or deserialization fails.
    ///
    /// # Panics
    ///
    /// Panics if `ctx_data` serializes to a non-object JSON value (e.g. an array
    /// or scalar). All call sites pass struct-typed contexts, so this is a
    /// programming-error invariant rather than a recoverable failure.
    pub fn call<R: DeserializeOwned>(
        &self,
        ctx: &HookContext,
    ) -> Result<R, Report<PluginSyncStateError>> {
        // 1. Extract the inner JSON value.
        let mut ctx_json = ctx.value().clone();

        // 2. Inject plugin_data from DashMap (snapshot at call time).
        //    Global plugins are scoped by name; attached plugins do not have
        //    sync hooks today (on_session_preview was removed from the judge),
        //    so the global read is sufficient here. Instance-scoped data is
        //    read via the async path (run_single_hook / build_async_ctx).
        if let Some(data) = self.plugin_data.get(&self.plugin_name) {
            let obj = ctx_json.as_object_mut().ok_or_else(|| {
                Report::new(PluginSyncStateError)
                    .attach("ctx_data did not serialize to a JSON object")
            })?;
            obj.insert("plugin_data".to_owned(), data);
        }

        // 3. Build the Lua ctx table.
        let ctx_table = build_sync_ctx(
            self.lua,
            &ctx_json,
            &self.plugin_name,
            None,
            self.plugin_data,
            &self.emit_tx,
            &self.in_flight,
        )
        .map_err(|e| Report::new(PluginSyncStateError).attach(e.to_string()))
        .attach("build ctx")?;

        // 4. Call the hook function.
        let result: Value = self
            .func
            .call(ctx_table)
            .map_err(|e| Report::new(PluginSyncStateError).attach(e.to_string()))
            .attach(format!("hook '{}'", self.plugin_name))?;

        // 5. Convert return value to JSON, then deserialize.
        let result_json = bindings::value_to_json(self.lua, &result)
            .map_err(|e| Report::new(PluginSyncStateError).attach(e.to_string()))
            .attach("convert return")?;

        serde_json::from_value(result_json)
            .change_context(PluginSyncStateError)
            .attach("deserialize return")
    }
}

impl SyncPlugins {
    /// Iterate over all plugins that define the given hook.
    ///
    /// Each yielded [`SyncHook`] can be called with context data. Plugins
    /// that don't define the hook are skipped.
    pub fn sync_hooks(&self, hook_name: &str) -> impl Iterator<Item = SyncHook<'_>> {
        let hook_name = hook_name.to_owned();
        self.hooks.iter().filter_map(move |(plugin_name, hooks)| {
            // Look up the function from the plugin's returned table.
            let table: mlua::Table = self.lua.registry_value(hooks.table()).ok()?;
            let val: Value = table.get(hook_name.as_str()).ok()?;

            match val {
                Value::Function(f) => Some(SyncHook::new(
                    &self.lua,
                    plugin_name.clone(),
                    f,
                    &self.plugin_data,
                    self.emit_tx.clone(),
                    &self.in_flight,
                )),
                _ => None,
            }
        })
    }

    /// Number of loaded plugins.
    pub fn plugin_count(&self) -> usize {
        self.hooks.len()
    }

    /// Create an empty SyncPlugins with no loaded plugins.
    ///
    /// Used as a default for tests that don't need plugin functionality.
    #[must_use]
    pub fn empty() -> Self {
        let (emit_tx, _emit_rx) = kanal::unbounded::<super::command::PluginCommand>();
        Self {
            lua: Lua::new(),
            hooks: HashMap::new(),
            plugin_data: PluginData::new(),
            emit_tx,
            in_flight: super::InFlightRequests::new(),
        }
    }

    /// Returns keybinds declared by all loaded plugins.
    ///
    /// Reads each plugin module's `keybinds` table and deserializes it into
    /// [`PluginKeybind`] records. Plugins that don't declare any keybinds, or
    /// whose `keybinds` field is missing/malformed, are skipped (malformed
    /// entries are logged at warn and dropped).
    #[must_use]
    pub fn declared_keybinds(&self) -> Vec<PluginKeybind> {
        self.hooks
            .iter()
            .flat_map(|(plugin_name, hooks)| {
                self.keybinds_for_plugin(plugin_name, hooks)
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Reads a single plugin's `keybinds` table.
    fn keybinds_for_plugin(
        &self,
        plugin_name: &str,
        hooks: &PluginHooks,
    ) -> Result<Vec<PluginKeybind>, Report<PluginSyncStateError>> {
        let table: mlua::Table = self
            .lua
            .registry_value(&hooks.table)
            .map_err(|e| Report::new(PluginSyncStateError).attach(e.to_string()))
            .attach("failed to read plugin table")?;
        let val: Value = table.get("keybinds").unwrap_or(Value::Nil);
        if val == Value::Nil {
            return Ok(Vec::new());
        }
        let arr: mlua::Table = match val {
            Value::Table(t) => t,
            other => {
                tracing::warn!(
                    plugin = plugin_name,
                    "plugin `keybinds` is not a table; skipping"
                );
                let _ = other;
                return Ok(Vec::new());
            }
        };
        let mut out = Vec::new();
        // Deserialize via mlua's serde support (`serialize` feature).
        for entry_result in arr.sequence_values::<Value>() {
            let entry = match entry_result {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(plugin = plugin_name, error = %e, "bad keybind entry; skipped");
                    continue;
                }
            };
            match self.lua.from_value::<PluginKeybindRaw>(entry) {
                Ok(raw) => out.push(raw.into_keybind(plugin_name.to_owned())),
                Err(e) => {
                    tracing::warn!(plugin = plugin_name, error = %e, "malformed keybind entry; skipped");
                }
            }
        }
        Ok(out)
    }
}

impl crate::feat::plugin_dispatch::PluginSyncHooks for SyncPlugins {
    fn call_hooks(&self, hook: &str, ctx: &HookContext) -> Vec<serde_json::Value> {
        self.sync_hooks(hook)
            .filter_map(|h| match h.call::<serde_json::Value>(ctx) {
                Ok(v) => (!v.is_null()).then_some(v),
                Err(e) => {
                    let report = e.attach(PluginHookSite {
                        plugin: h.plugin_name().to_owned(),
                        hook: hook.to_owned(),
                    });
                    tracing::error!(hook, error = ?report, "plugin hook failed");
                    None
                }
            })
            .collect()
    }
}

/// Build the ctx table for a sync hook call.
///
/// Includes data fields from `ctx_json`, `plugin_data`, and `ctx.emit()`.
/// Does NOT include `ctx.request()` (would block render thread).
pub(crate) fn build_sync_ctx(
    lua: &Lua,
    ctx_json: &serde_json::Value,
plugin_name: &str,
instance_id: Option<&str>,
    plugin_data: &PluginData,
    emit_tx: &kanal::Sender<PluginCommand>,
    in_flight: &super::InFlightRequests,
) -> Result<mlua::Table, mlua::Error> {
    let ctx = lua.create_table()?;

    // Set data fields from JSON (flattened into ctx top-level).
    if let Some(obj) = ctx_json.as_object() {
        for (k, v) in obj {
            ctx.set(k.as_str(), bindings::json_to_lua_value(lua, v)?)?;
        }
    }

    // ctx.plugin_name — let Lua refer to itself by name.
    ctx.set("plugin_name", plugin_name)?;

    // ctx.instance_id — stable unique identity of this attachment
    // (nil for global plugins). Set alongside plugin_name so hooks and
    // tool handlers know which instance they are running for.
    if let Some(id) = instance_id {
        ctx.set("instance_id", id)?;
    }
    ctx.set("plugin_name", plugin_name)?;

    // ctx.emit(cmd, data) — fire-and-forget via channel.
    let emit_tx = emit_tx.clone();
    let emit_pname = plugin_name.to_owned();
    let emit_fn = lua.create_function(move |lua, (name, data): (String, Value)| {
        let json = bindings::value_to_json(lua, &data).unwrap_or_default();
        let _ = emit_tx.send(PluginCommand {
            plugin_name: emit_pname.clone(),
            name,
            data: json,
        });
        Ok(())
    })?;
    ctx.set("emit", emit_fn)?;

    // ctx.cancel(task) — fire an in-flight async request's token.
    //
    // Sync-safe: just fires the token (no .await). The cancelled request's
    // spawned future observes the cancellation via its `select!` arm and
    // runs cleanup (e.g. `CancelStream`) there.
    {
        let in_flight = in_flight.clone();
        let cancel_fn = lua.create_function(move |_, task: String| {
            tracing::debug!(task = %task, "sync ctx.cancel: firing token");
            in_flight.cancel(&task);
            Ok(())
        })?;
        ctx.set("cancel", cancel_fn)?;
    }
    // ctx.set_plugin_data(value) — writes to shared DashMap.
    //
    // Sync-safe: PluginData is an Arc<DashMap>, writable from any thread.
    // Sync hooks get the same write capabilities as async hooks; this unlocks
    // plugins that need to manage state from a sync hook (e.g. cancel decisions
    // in on_keybind_trigger). The session_id for scoping is extracted from ctx_json.
    {
        let pd = plugin_data.clone();
        let pname = plugin_name.to_owned();
        let set_data_fn = lua.create_function(move |lua, value: mlua::Value| {
            let json = bindings::value_to_json(lua, &value).unwrap_or_default();
            pd.set(&pname, json);
            Ok(())
        })?;
        ctx.set("set_plugin_data", set_data_fn)?;
    }

    {
        let pd = plugin_data.clone();
        let pname = plugin_name.to_owned();
        let merge_data_fn = lua.create_function(move |lua, value: mlua::Value| {
            let json = bindings::value_to_json(lua, &value).unwrap_or_default();
            pd.merge(&pname, json);
            Ok(())
        })?;
        ctx.set("merge_plugin_data", merge_data_fn)?;
    }

    {
        let pd = plugin_data.clone();
        let pname = plugin_name.to_owned();
        let get_data_fn = lua.create_function(move |lua, (): ()| {
            let json = pd.get(&pname).unwrap_or_else(|| serde_json::json!({}));
            bindings::json_to_lua_value(lua, &json)
        })?;
        ctx.set("get_plugin_data", get_data_fn)?;
    }

    Ok(ctx)
}

/// Extract a SessionId from a sync hook's ctx JSON (the session_id field).
/// Returns None for global plugin hooks that don't carry a session ID.
fn extract_sync_session_id(ctx_json: &serde_json::Value) -> Option<SessionId> {
    ctx_json
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| SessionId::from(s.to_owned()))
}
