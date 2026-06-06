#![expect(dead_code, reason = "sync hook caching not yet wired")]
//! Sync-side plugin handle — owns the Lua state that runs on the render thread.
//!
//! [`SyncPlugins`] is `!Send` because `mlua::Lua` is `!Send`. It must live
//! on the render thread. Sync hooks are called directly, with no thread hops.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;

use error_stack::{Report, ResultExt};
use mlua::{Lua, RegistryKey, Value};
use serde::Serialize;
use serde::de::DeserializeOwned;
use wherror::Error;

use crate::PluginData;
use crate::bindings;
use crate::command::PluginCommand;

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
    ) -> Self {
        Self {
            lua,
            hooks,
            plugin_data,
            emit_tx,
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
        Self::new(Lua::new(), HashMap::new(), PluginData::new(), emit_tx)
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
    /// Channel for ctx.emit().
    emit_tx: kanal::Sender<PluginCommand>,
}

impl<'a> SyncHook<'a> {
    /// Construct a single sync hook ready to be called.
    pub(crate) fn new(
        lua: &'a Lua,
        plugin_name: String,
        func: mlua::Function,
        plugin_data: &'a PluginData,
        emit_tx: kanal::Sender<PluginCommand>,
    ) -> Self {
        Self {
            lua,
            plugin_name,
            func,
            plugin_data,
            emit_tx,
        }
    }
}

impl SyncHook<'_> {
    /// Call this hook with context data and deserialize the return value.
    ///
    /// # Type Parameters
    ///
    /// - `T` — the context struct (must be `Serialize`)
    /// - `R` — the expected return type (must be `DeserializeOwned`)
    /// # Errors
    /// Returns `Err` if serialization, Lua execution, or deserialization fails.
    pub fn call<T: Serialize, R: DeserializeOwned>(
        &self,
        ctx_data: &T,
    ) -> Result<R, Report<PluginSyncStateError>> {
        // 1. Serialize ctx_data to JSON.
        let mut ctx_json = serde_json::to_value(ctx_data)
            .change_context(PluginSyncStateError)
            .attach("serialize ctx")?;

        // 2. Inject plugin_data from DashMap (snapshot at call time).
        if let Some(data) = self.plugin_data.get(&self.plugin_name) {
            let obj = ctx_json.as_object_mut().ok_or_else(|| {
                Report::new(PluginSyncStateError)
                    .attach("ctx_data did not serialize to a JSON object")
            })?;
            obj.insert("plugin_data".to_owned(), data.clone());
        }

        // 3. Build the Lua ctx table.
        let ctx_table = build_sync_ctx(
            self.lua,
            &ctx_json,
            &self.plugin_name,
            self.plugin_data,
            &self.emit_tx,
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
                )),
                _ => None,
            }
        })
    }

    /// Create an empty SyncPlugins with no loaded plugins.
    ///
    /// Used as a default for tests that don't need plugin functionality.
    #[must_use]
    pub fn empty() -> Self {
        let (emit_tx, _emit_rx) = kanal::unbounded::<crate::command::PluginCommand>();
        Self {
            lua: Lua::new(),
            hooks: HashMap::new(),
            plugin_data: PluginData::new(),
            emit_tx,
        }
    }
    /// Returns the number of loaded plugins.
    #[must_use]
    pub fn plugin_count(&self) -> usize {
        self.hooks.len()
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
    plugin_data: &PluginData,
    emit_tx: &kanal::Sender<PluginCommand>,
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

    // ctx.emit(cmd, data) — fire-and-forget via channel.
    let emit_tx = emit_tx.clone();
    let plugin_name = plugin_name.to_owned();
    let emit_fn = lua.create_function(move |lua, (name, data): (String, Value)| {
        let json = bindings::value_to_json(lua, &data).unwrap_or_default();
        let _ = emit_tx.send(PluginCommand {
            plugin_name: plugin_name.clone(),
            name,
            data: json,
        });
        Ok(())
    })?;
    ctx.set("emit", emit_fn)?;

    // NO ctx.request() — sync hooks can't do async I/O.
    // NO ctx.set_plugin_data() — sync hooks don't write persistent data.
    // (If needed, these can be added to the async ctx only.)

    let _ = plugin_data;
    Ok(ctx)
}
