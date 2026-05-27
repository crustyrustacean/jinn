//! Centralized plugin registry — owns one sandboxed Lua VM per plugin.
//!
//! [`PluginRegistry`] is the single entry point for all plugin operations.
//! Each plugin gets its own Lua VM so global state never leaks between plugins.
//!
//! The registry is `!Send` (because `mlua::Lua` is `!Send`) and must live
//! on the main TUI thread.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use error_stack::Report;
use mlua::Lua;
use wherror::Error;

use crate::loader;
use crate::translator::TranslatorFn;

// ── CommandSender ───────────────────────��──────────────────────────────────

/// Callback for sending commands from Lua into the application.
///
/// The TUI wiring provides an implementation that wraps the command in
/// `AppMsg` and sends it through the kanal channel.
#[derive(Clone)]
pub struct CommandSender {
    /// The inner callback.
    inner: Arc<dyn Fn(nullslop_domain::Command) + Send + Sync>,
}

impl CommandSender {
    /// Creates a new command sender from a callback.
    pub fn new<F>(sender: F) -> Self
    where
        F: Fn(nullslop_domain::Command) + Send + Sync + 'static,
    {
        Self {
            inner: Arc::new(sender),
        }
    }

    /// Sends a command through the callback.
    pub fn send(&self, cmd: nullslop_domain::Command) {
        (self.inner)(cmd);
    }
}

// ── PluginInfo ─────────────────────────────────────────────────────────────

/// Metadata about a loaded plugin.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    /// The plugin directory name (e.g., `"welcome"`).
    pub name: String,
    /// The path to the plugin directory.
    pub path: PathBuf,
}

// ── PluginError ────────────────────────────────────────────────────────────

/// Errors from plugin loading.
#[derive(Debug, Error)]
#[error(debug)]
pub struct PluginError;

// ── PluginInstance ─────────────────────────────────────────────────────────

/// One sandboxed Lua VM per plugin.
///
/// Each instance owns its own VM, subscription map, and hook map.
/// No state is shared between instances.
pub struct PluginInstance {
    /// Plugin directory name.
    name: String,
    /// The sandboxed Lua VM.
    lua: Lua,
    /// Fire-and-forget subscriptions (`ps.sub`).
    subscriptions: Arc<RefCell<std::collections::HashMap<String, Vec<mlua::Function>>>>,
    /// Data-returning hooks (`ps.hook`).
    hooks: Arc<RefCell<std::collections::HashMap<String, Vec<mlua::Function>>>>,
}

impl PluginInstance {
    /// Creates a new plugin instance with a fresh Lua VM.
    ///
    /// Installs the `ns` and `ps` globals into the VM. The subscription
    /// and hook maps are populated by the `ps.sub` and `ps.hook` bindings
    /// as the plugin loads.
    fn new(
        name: String,
        translator: TranslatorFn,
        command_sender: CommandSender,
    ) -> Result<Self, Report<PluginError>> {
        let lua = Lua::new();

        #[expect(
            clippy::arc_with_non_send_sync,
            reason = "mlua::Function is !Send but only used from the main thread"
        )]
        let subscriptions: Arc<RefCell<std::collections::HashMap<String, Vec<mlua::Function>>>> =
            Arc::new(RefCell::new(std::collections::HashMap::new()));

        #[expect(
            clippy::arc_with_non_send_sync,
            reason = "mlua::Function is !Send but only used from the main thread"
        )]
        let hooks: Arc<RefCell<std::collections::HashMap<String, Vec<mlua::Function>>>> =
            Arc::new(RefCell::new(std::collections::HashMap::new()));

        // Install ns table.
        let ns = lua
            .create_table()
            .map_err(|e| {
                tracing::error!(err = %e, "failed to create ns table");
                Report::new(PluginError).attach("Lua table creation failed")
            })?;

        {
            let translator = translator.clone();
            let sender = command_sender.clone();
            let ns_emit = lua
                .create_function(move |lua, (name, payload): (String, mlua::Value)| {
                    let json_payload = match crate::bindings::value_to_json(lua, &payload) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!(err = %e, "failed to convert ns.emit payload");
                            return Ok(());
                        }
                    };

                    match translator(name.as_str(), json_payload) {
                        Some(cmd) => sender.send(cmd),
                        None => {
                            tracing::warn!(
                                name,
                                "unknown plugin command, no translator registered"
                            );
                        }
                    }
                    Ok(())
                })
                .map_err(|e| {
                    tracing::error!(err = %e, "failed to create ns.emit function");
                    Report::new(PluginError).attach("Lua function creation failed")
                })?;
            ns.set("emit", ns_emit).map_err(|e| {
                tracing::error!(err = %e, "failed to set ns.emit");
                Report::new(PluginError).attach("Lua table set failed")
            })?;
        }

        // Install ps table.
        let ps = lua
            .create_table()
            .map_err(|e| {
                tracing::error!(err = %e, "failed to create ps table");
                Report::new(PluginError).attach("Lua table creation failed")
            })?;

        // ps.sub — fire-and-forget subscription.
        {
            let subs = subscriptions.clone();
            let ps_sub = lua
                .create_function(
                    move |_lua, (name, callback): (String, mlua::Function)| {
                        let mut map = subs.borrow_mut();
                        map.entry(name).or_default().push(callback);
                        Ok(())
                    },
                )
                .map_err(|e| {
                    tracing::error!(err = %e, "failed to create ps.sub function");
                    Report::new(PluginError).attach("Lua function creation failed")
                })?;
            ps.set("sub", ps_sub).map_err(|e| {
                tracing::error!(err = %e, "failed to set ps.sub");
                Report::new(PluginError).attach("Lua table set failed")
            })?;
        }

        // ps.hook — data-returning hook registration.
        {
            let hooks_ref = hooks.clone();
            let ps_hook = lua
                .create_function(
                    move |_lua, (name, callback): (String, mlua::Function)| {
                        let mut map = hooks_ref.borrow_mut();
                        map.entry(name).or_default().push(callback);
                        Ok(())
                    },
                )
                .map_err(|e| {
                    tracing::error!(err = %e, "failed to create ps.hook function");
                    Report::new(PluginError).attach("Lua function creation failed")
                })?;
            ps.set("hook", ps_hook).map_err(|e| {
                tracing::error!(err = %e, "failed to set ps.hook");
                Report::new(PluginError).attach("Lua table set failed")
            })?;
        }

        // ps.pub — plugin-internal publish (fires ps.sub callbacks in same VM).
        {
            let subs = subscriptions.clone();
            let ps_pub = lua
                .create_function(move |_lua, (name, payload): (String, mlua::Value)| {
                    let callbacks: Vec<mlua::Function> = {
                        let map = subs.borrow();
                        map.get(&name).cloned().unwrap_or_default()
                    };
                    for callback in &callbacks {
                        if let Err(e) = callback.call::<()>(payload.clone()) {
                            tracing::warn!(err = %e, "ps.pub callback error");
                        }
                    }
                    Ok(())
                })
                .map_err(|e| {
                    tracing::error!(err = %e, "failed to create ps.pub function");
                    Report::new(PluginError).attach("Lua function creation failed")
                })?;
            ps.set("pub", ps_pub).map_err(|e| {
                tracing::error!(err = %e, "failed to set ps.pub");
                Report::new(PluginError).attach("Lua table set failed")
            })?;
        }

        // ps.unsub — unsubscribe from both subscriptions and hooks.
        {
            let subs = subscriptions.clone();
            let hooks_ref = hooks.clone();
            let ps_unsub = lua
                .create_function(move |_lua, name: String| {
                    subs.borrow_mut().remove(&name);
                    hooks_ref.borrow_mut().remove(&name);
                    Ok(())
                })
                .map_err(|e| {
                    tracing::error!(err = %e, "failed to create ps.unsub function");
                    Report::new(PluginError).attach("Lua function creation failed")
                })?;
            ps.set("unsub", ps_unsub).map_err(|e| {
                tracing::error!(err = %e, "failed to set ps.unsub");
                Report::new(PluginError).attach("Lua table set failed")
            })?;
        }

        lua.globals().set("ns", ns).map_err(|e| {
            tracing::error!(err = %e, "failed to set ns global");
            Report::new(PluginError).attach("Lua global set failed")
        })?;
        lua.globals().set("ps", ps).map_err(|e| {
            tracing::error!(err = %e, "failed to set ps global");
            Report::new(PluginError).attach("Lua global set failed")
        })?;

        Ok(Self {
            name,
            lua,
            subscriptions,
            hooks,
        })
    }
}

// ── PluginRegistry ─────────────────────────────────────────────────────────

/// Centralized plugin registry — owns all plugin VMs.
///
/// Each loaded plugin runs in its own sandboxed Lua VM. The registry
/// provides [`emit`](Self::emit) for fire-and-forget events and
/// [`for_hook`](Self::for_hook) for data-returning hooks.
///
/// The registry is `!Send` because it owns `mlua::Lua` instances.
/// It must be created and used on the main TUI thread.
pub struct PluginRegistry {
    /// Translator callback: command name + JSON → Option<Command>.
    translator: TranslatorFn,
    /// Command sender for injecting typed commands.
    command_sender: CommandSender,
    /// One VM per loaded plugin.
    instances: RefCell<Vec<PluginInstance>>,
    /// Names of plugins that have been loaded (dedup).
    loaded: RefCell<HashSet<String>>,
}

impl PluginRegistry {
    /// Creates a new registry with a translator and command sender.
    pub fn new(translator: TranslatorFn, sender: CommandSender) -> Self {
        Self {
            translator,
            command_sender: sender,
            instances: RefCell::new(Vec::new()),
            loaded: RefCell::new(HashSet::new()),
        }
    }

    /// Creates a registry suitable for tests (no-op translator, no-op sender).
    pub fn new_for_tests() -> Self {
        let translator = crate::translator::noop_translator();
        let sender = CommandSender::new(|_cmd| {});
        Self::new(translator, sender)
    }

    /// Loads a single plugin from a directory containing `init.lua`.
    ///
    /// Creates a new sandboxed Lua VM, installs bindings, and executes
    /// `init.lua`. Returns metadata about the loaded plugin.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory doesn't exist, `init.lua` is
    /// missing, or the Lua script fails to execute.
    pub fn load_plugin(&self, dir: &Path) -> Result<PluginInfo, Report<PluginError>> {
        let init_path = dir.join("init.lua");
        if !init_path.is_file() {
            return Err(Report::new(PluginError)
                .attach(format!("no init.lua in {}", dir.display())));
        }

        let name = dir
            .file_name()
            .map_or_else(|| String::from("unknown"), |n| n.to_string_lossy().into_owned());

        if self.loaded.borrow().contains(&name) {
            tracing::warn!(plugin = %name, "skipping plugin: already loaded");
            return Err(Report::new(PluginError)
                .attach(format!("plugin '{name}' is already loaded")));
        }

        let instance = PluginInstance::new(
            name.clone(),
            self.translator.clone(),
            self.command_sender.clone(),
        )?;

        let source = std::fs::read_to_string(&init_path).map_err(|e| {
            tracing::error!(err = %e, path = %init_path.display(), "failed to read init.lua");
            Report::new(PluginError).attach(format!("read error: {}", init_path.display()))
        })?;

        instance
            .lua
            .load(&source)
            .set_name(format!("plugin/{name}/init.lua"))
            .exec()
            .map_err(|e| {
                tracing::error!(err = %e, plugin = %name, "plugin init.lua failed");
                Report::new(PluginError).attach(format!("plugin '{name}' failed to load"))
            })?;

        tracing::info!(plugin = %name, "loaded plugin");

        self.loaded.borrow_mut().insert(name.clone());
        self.instances.borrow_mut().push(instance);

        Ok(PluginInfo {
            name,
            path: dir.to_path_buf(),
        })
    }

    /// Loads all plugins from a directory.
    ///
    /// Scans for subdirectories containing `init.lua`, loads each one into
    /// its own VM, and logs warnings for any that fail. Returns info for
    /// successfully loaded plugins.
    pub fn load_all(&self, plugins_dir: &Path) -> Vec<PluginInfo> {
        let dirs = match loader::scan(plugins_dir) {
            Ok(dirs) => dirs,
            Err(e) => {
                tracing::warn!(err = %e, "failed to scan plugins directory");
                return Vec::new();
            }
        };

        let mut loaded = Vec::new();
        for dir in dirs {
            match self.load_plugin(&dir) {
                Ok(info) => loaded.push(info),
                Err(e) => {
                    tracing::warn!(err = %e, "skipping plugin");
                }
            }
        }

        loaded
    }

    /// Fire-and-forget event dispatch to all VMs.
    ///
    /// Calls every `ps.sub` callback registered for `event_name` across
    /// all plugin instances. Individual callback errors are logged as
    /// warnings and do not stop dispatch to other VMs or callbacks.
    pub fn emit(&self, event_name: &str, payload: &serde_json::Value) {
        let instances = self.instances.borrow();
        for instance in instances.iter() {
            let callbacks: Vec<mlua::Function> = {
                let map = instance.subscriptions.borrow();
                map.get(event_name).cloned().unwrap_or_default()
            };

            if callbacks.is_empty() {
                continue;
            }

            let lua_payload =
                match crate::bindings::json_to_lua_value(&instance.lua, payload) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(
                            err = %e,
                            plugin = %instance.name,
                            "failed to convert payload to Lua"
                        );
                        continue;
                    }
                };

            for callback in &callbacks {
                if let Err(e) = callback.call::<()>(lua_payload.clone()) {
                    tracing::warn!(
                        err = %e,
                        plugin = %instance.name,
                        event_name,
                        "subscriber callback error"
                    );
                }
            }
        }
    }

    /// Data-returning hook call to all VMs.
    ///
    /// Calls every `ps.hook` callback registered for `hook_name` across
    /// all plugin instances. Each callback's return value is deserialized
    /// into `T`. Individual failures (Lua errors or deserialization errors)
    /// are logged as warnings and excluded from results.
    pub fn for_hook<T>(&self, hook_name: &str, payload: &serde_json::Value) -> Vec<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut results = Vec::new();
        let instances = self.instances.borrow();

        for instance in instances.iter() {
            let callbacks: Vec<mlua::Function> = {
                let map = instance.hooks.borrow();
                map.get(hook_name).cloned().unwrap_or_default()
            };

            if callbacks.is_empty() {
                continue;
            }

            let lua_payload =
                match crate::bindings::json_to_lua_value(&instance.lua, payload) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(
                            err = %e,
                            plugin = %instance.name,
                            "failed to convert payload to Lua"
                        );
                        continue;
                    }
                };

            for callback in &callbacks {
                match callback.call::<mlua::Value>(lua_payload.clone()) {
                    Ok(return_value) => {
                        let json_value =
                            match crate::bindings::value_to_json(&instance.lua, &return_value) {
                                Ok(v) => v,
                                Err(e) => {
                                    tracing::warn!(
                                        err = %e,
                                        plugin = %instance.name,
                                        "failed to convert hook return value"
                                    );
                                    continue;
                                }
                            };

                        match serde_json::from_value::<T>(json_value) {
                            Ok(item) => results.push(item),
                            Err(e) => {
                                tracing::warn!(
                                    err = %e,
                                    plugin = %instance.name,
                                    hook_name,
                                    "hook returned data that failed deserialization"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            err = %e,
                            plugin = %instance.name,
                            hook_name,
                            "hook callback error"
                        );
                    }
                }
            }
        }

        results
    }

    /// Returns the number of loaded plugins.
    #[must_use]
    pub fn len(&self) -> usize {
        self.instances.borrow().len()
    }

    /// Returns `true` if no plugins are loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instances.borrow().is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unwrap_in_result,
        reason = "test code, panics are acceptable"
    )]

    use nullslop_domain::Command;
    use std::fs;

    use super::*;

    fn test_sender() -> (CommandSender, kanal::Receiver<Command>) {
        let (tx, rx) = kanal::unbounded();
        let sender = CommandSender::new(move |cmd: Command| {
            let _ = tx.send(cmd);
        });
        (sender, rx)
    }

    fn test_registry() -> PluginRegistry {
        PluginRegistry::new_for_tests()
    }

    fn test_registry_with_sender() -> (PluginRegistry, kanal::Receiver<Command>) {
        let (sender, rx) = test_sender();
        let translator = crate::translator::noop_translator();
        (PluginRegistry::new(translator, sender), rx)
    }

    // ── Construction ───────────────────────────────────────────────────

    #[rstest::rstest]
    fn new_for_tests_creates_registry() {
        // Given no special setup.
        // When creating a test registry.
        let registry = test_registry();

        // Then it is empty.
        assert!(registry.is_empty());
    }

    // ── Loading plugins ────────────────────────────────────────────────

    #[rstest::rstest]
    fn load_plugin_from_valid_dir_succeeds() {
        // Given a temp directory with a valid init.lua.
        let registry = test_registry();
        let dir = tempfile::tempdir().expect("create temp dir");
        let plugin_dir = dir.path().join("test-plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(plugin_dir.join("init.lua"), "-- test plugin").expect("write init.lua");

        // When loading the plugin.
        let result = registry.load_plugin(&plugin_dir);

        // Then it succeeds with correct info.
        let info = result.expect("load should succeed");
        assert_eq!(info.name, "test-plugin");
        assert_eq!(registry.len(), 1);
    }

    #[rstest::rstest]
    fn load_plugin_from_missing_dir_returns_error() {
        // Given a nonexistent path.
        let registry = test_registry();

        // When loading from a nonexistent directory.
        let result = registry.load_plugin(Path::new("/nonexistent/path"));

        // Then it returns an error.
        assert!(result.is_err(), "should fail for missing directory");
    }

    #[rstest::rstest]
    fn load_plugin_from_dir_without_init_returns_error() {
        // Given a directory without init.lua.
        let registry = test_registry();
        let dir = tempfile::tempdir().expect("create temp dir");

        // When loading from a directory without init.lua.
        let result = registry.load_plugin(dir.path());

        // Then it returns an error.
        assert!(result.is_err(), "should fail without init.lua");
    }

    #[rstest::rstest]
    fn load_all_loads_multiple_plugins() {
        // Given a plugins directory with 3 valid plugins.
        let registry = test_registry();
        let dir = tempfile::tempdir().expect("create temp dir");
        for name in ["alpha", "beta", "gamma"] {
            let plugin_dir = dir.path().join(name);
            fs::create_dir_all(&plugin_dir).expect("create dir");
            fs::write(plugin_dir.join("init.lua"), format!("-- {name}")).expect("write init.lua");
        }

        // When loading all plugins.
        let loaded = registry.load_all(dir.path());

        // Then 3 plugins are loaded.
        assert_eq!(loaded.len(), 3);
        let names: Vec<&str> = loaded.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
        assert!(names.contains(&"gamma"));
    }

    #[rstest::rstest]
    fn load_all_skips_invalid_and_logs_warning() {
        // Given a plugins directory with 2 valid and 1 invalid plugin.
        let registry = test_registry();
        let dir = tempfile::tempdir().expect("create temp dir");

        // Valid plugins.
        for name in ["valid1", "valid2"] {
            let plugin_dir = dir.path().join(name);
            fs::create_dir_all(&plugin_dir).expect("create dir");
            fs::write(plugin_dir.join("init.lua"), format!("-- {name}"))
                .expect("write init.lua");
        }

        // Invalid: dir with no init.lua.
        let invalid = dir.path().join("broken");
        fs::create_dir_all(&invalid).expect("create dir");

        // When loading all plugins.
        let loaded = registry.load_all(dir.path());

        // Then 2 plugins are loaded, no panic.
        assert_eq!(loaded.len(), 2);
    }

    #[rstest::rstest]
    fn load_plugin_skips_duplicate_name() {
        // Given two temp directories with the same plugin directory name.
        let registry = test_registry();

        let dir1 = tempfile::tempdir().expect("create temp dir 1");
        let plugin_dir1 = dir1.path().join("dup");
        fs::create_dir_all(&plugin_dir1).expect("create plugin dir 1");
        fs::write(plugin_dir1.join("init.lua"), "-- first").expect("write init.lua 1");

        let dir2 = tempfile::tempdir().expect("create temp dir 2");
        let plugin_dir2 = dir2.path().join("dup");
        fs::create_dir_all(&plugin_dir2).expect("create plugin dir 2");
        fs::write(plugin_dir2.join("init.lua"), "-- second").expect("write init.lua 2");

        // When loading both.
        let first = registry.load_plugin(&plugin_dir1);
        let second = registry.load_plugin(&plugin_dir2);

        // Then the first succeeds and the second is rejected.
        assert!(first.is_ok(), "first load should succeed");
        assert!(second.is_err(), "duplicate should be rejected");
    }

    #[rstest::rstest]
    fn load_all_skips_already_loaded_plugin() {
        // Given a registry with one plugin already loaded.
        let registry = test_registry();

        let dir1 = tempfile::tempdir().expect("create temp dir 1");
        let plugin_dir1 = dir1.path().join("alpha");
        fs::create_dir_all(&plugin_dir1).expect("create plugin dir 1");
        fs::write(plugin_dir1.join("init.lua"), "-- first").expect("write init.lua 1");

        let result = registry.load_plugin(&plugin_dir1);
        assert!(result.is_ok(), "initial load should succeed");

        // And a second directory also containing "alpha".
        let dir2 = tempfile::tempdir().expect("create temp dir 2");
        let plugin_dir2 = dir2.path().join("alpha");
        fs::create_dir_all(&plugin_dir2).expect("create plugin dir 2");
        fs::write(plugin_dir2.join("init.lua"), "-- second").expect("write init.lua 2");
        // Also add a new plugin "beta".
        let plugin_dir3 = dir2.path().join("beta");
        fs::create_dir_all(&plugin_dir3).expect("create plugin dir 3");
        fs::write(plugin_dir3.join("init.lua"), "-- beta").expect("write init.lua 3");

        // When loading all from the second directory.
        let loaded = registry.load_all(dir2.path());

        // Then only beta is loaded (alpha was skipped as duplicate).
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "beta");
    }

    // ── Per-VM isolation ───────────────────────────────────────────────

    #[rstest::rstest]
    fn each_plugin_has_separate_globals() {
        // Given a registry with two plugins that set global variables.
        let registry = test_registry();
        let dir = tempfile::tempdir().expect("create temp dir");

        let plugin_a = dir.path().join("plugin-a");
        fs::create_dir_all(&plugin_a).expect("create dir");
        fs::write(plugin_a.join("init.lua"), "shared_var = 'from_a'").expect("write init.lua");

        let plugin_b = dir.path().join("plugin-b");
        fs::create_dir_all(&plugin_b).expect("create dir");
        fs::write(plugin_b.join("init.lua"), "shared_var = 'from_b'").expect("write init.lua");

        registry.load_all(dir.path());

        // When reading the global from each instance.
        let instances = registry.instances.borrow();
        let a_val: String = instances[0]
            .lua
            .globals()
            .get("shared_var")
            .expect("get shared_var from a");
        let b_val: String = instances[1]
            .lua
            .globals()
            .get("shared_var")
            .expect("get shared_var from b");

        // Then each plugin has its own value.
        assert_eq!(a_val, "from_a");
        assert_eq!(b_val, "from_b");
    }

    // ── emit (fire-and-forget) ─────────────────────────────────────────

    #[rstest::rstest]
    fn emit_fires_subscriber_callback() {
        // Given a registry with a Lua plugin that subscribes to an event.
        let dir = tempfile::tempdir().expect("create temp dir");
        let plugin_dir = dir.path().join("sub-plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");

        // Use a translator that recognizes "test_cmd" and produces a command.
        let (tx_test, rx_test) = kanal::unbounded();
        let translator: TranslatorFn = Arc::new(move |name, _payload| {
            if name == "test_cmd" {
                let tx = tx_test.clone();
                tx.send(true).ok();
            }
            None
        });
        let (sender, _rx) = test_sender();
        let registry = PluginRegistry::new(translator, sender);

        // Plugin subscribes to "test::event" and emits a command when it fires.
        fs::write(
            plugin_dir.join("init.lua"),
            r#"
                ps.sub("test::event", function(payload)
                    ns.emit("test_cmd", { fired = true })
                end)
            "#,
        )
        .expect("write init.lua");
        registry.load_plugin(&plugin_dir).expect("load plugin");

        // When dispatching the event.
        registry.emit("test::event", &serde_json::json!({}));

        // Then the subscriber callback fired (translator was called).
        let result = rx_test.recv_timeout(std::time::Duration::from_millis(100));
        assert!(
            result.is_ok(),
            "emit should have triggered the subscriber"
        );
        drop(rx_test);
    }

    #[rstest::rstest]
    fn emit_to_unsubscribed_event_does_nothing() {
        // Given a registry with no subscribers for "other::event".
        let (registry, rx) = test_registry_with_sender();

        // When dispatching an event nobody subscribes to.
        registry.emit("other::event", &serde_json::json!({}));

        // Then no command is sent.
        let cmd = rx.recv_timeout(std::time::Duration::from_millis(50));
        assert!(cmd.is_err(), "no subscriber should mean no command");
    }

    #[rstest::rstest]
    fn emit_only_fires_sub_callbacks_not_hook_callbacks() {
        // Given a registry with a plugin that uses both ps.sub and ps.hook
        // for the same event name.
        let registry = test_registry();
        let dir = tempfile::tempdir().expect("create temp dir");
        let plugin_dir = dir.path().join("mixed-plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(
            plugin_dir.join("init.lua"),
            r#"
                sub_called = false
                hook_called = false
                ps.sub("test::evt", function(payload)
                    sub_called = true
                end)
                ps.hook("test::evt", function(payload)
                    hook_called = true
                    return { ok = true }
                end)
            "#,
        )
        .expect("write init.lua");
        registry.load_plugin(&plugin_dir).expect("load plugin");

        // When emitting the event.
        registry.emit("test::evt", &serde_json::json!({}));

        // Then only the sub callback was called, not the hook.
        let instances = registry.instances.borrow();
        let sub_called: bool = instances[0]
            .lua
            .globals()
            .get("sub_called")
            .expect("get sub_called");
        let hook_called: bool = instances[0]
            .lua
            .globals()
            .get("hook_called")
            .expect("get hook_called");
        assert!(sub_called, "ps.sub callback should have been called");
        assert!(!hook_called, "ps.hook callback should NOT have been called by emit");
    }

    // ── for_hook (data-returning) ──────────────────────────────────────

    #[rstest::rstest]
    fn for_hook_collects_return_values() {
        // Given a registry with a plugin that registers a hook returning data.
        let registry = test_registry();
        let dir = tempfile::tempdir().expect("create temp dir");
        let plugin_dir = dir.path().join("hook-plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(
            plugin_dir.join("init.lua"),
            r#"
                ps.hook("get_items", function(payload)
                    return { name = "test_item", count = 42 }
                end)
            "#,
        )
        .expect("write init.lua");
        registry.load_plugin(&plugin_dir).expect("load plugin");

        // When calling for_hook.
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct TestItem {
            name: String,
            count: i64,
        }
        let results: Vec<TestItem> = registry.for_hook("get_items", &serde_json::json!({}));

        // Then the hook return value was collected.
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "test_item");
        assert_eq!(results[0].count, 42);
    }

    #[rstest::rstest]
    fn for_hook_returns_empty_when_no_hooks_registered() {
        // Given a registry with no hooks for "nonexistent".
        let registry = test_registry();

        // When calling for_hook.
        let results: Vec<serde_json::Value> =
            registry.for_hook("nonexistent", &serde_json::json!({}));

        // Then no results.
        assert!(results.is_empty());
    }

    #[rstest::rstest]
    fn for_hook_only_fires_hook_callbacks_not_sub_callbacks() {
        // Given a registry with a plugin that uses both ps.sub and ps.hook.
        let registry = test_registry();
        let dir = tempfile::tempdir().expect("create temp dir");
        let plugin_dir = dir.path().join("mixed-plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(
            plugin_dir.join("init.lua"),
            r#"
                sub_called = false
                hook_called = false
                ps.sub("test::evt", function(payload)
                    sub_called = true
                end)
                ps.hook("test::evt", function(payload)
                    hook_called = true
                    return { ok = true }
                end)
            "#,
        )
        .expect("write init.lua");
        registry.load_plugin(&plugin_dir).expect("load plugin");

        // When calling for_hook.
        let _results: Vec<serde_json::Value> =
            registry.for_hook("test::evt", &serde_json::json!({}));

        // Then only the hook callback was called, not the sub.
        let instances = registry.instances.borrow();
        let sub_called: bool = instances[0]
            .lua
            .globals()
            .get("sub_called")
            .expect("get sub_called");
        let hook_called: bool = instances[0]
            .lua
            .globals()
            .get("hook_called")
            .expect("get hook_called");
        assert!(!sub_called, "ps.sub callback should NOT have been called by for_hook");
        assert!(hook_called, "ps.hook callback should have been called");
    }

    #[rstest::rstest]
    fn for_hook_logs_warning_on_bad_return_data() {
        // Given a registry with a plugin that returns data that won't
        // deserialize to the expected type.
        let registry = test_registry();
        let dir = tempfile::tempdir().expect("create temp dir");
        let plugin_dir = dir.path().join("bad-hook-plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(
            plugin_dir.join("init.lua"),
            r#"
                ps.hook("get_number", function(payload)
                    return "not_a_number"
                end)
            "#,
        )
        .expect("write init.lua");
        registry.load_plugin(&plugin_dir).expect("load plugin");

        // When calling for_hook expecting an integer.
        let results: Vec<i64> = registry.for_hook("get_number", &serde_json::json!({}));

        // Then no results (bad data excluded).
        assert!(results.is_empty(), "bad return data should be excluded");
    }

    // ── ps.unsub ──────────────────────────────────────────────────────

    #[rstest::rstest]
    fn unsub_stops_sub_delivery() {
        // Given a registry with a plugin that subscribes then unsubscribes.
        let registry = test_registry();
        let dir = tempfile::tempdir().expect("create temp dir");
        let plugin_dir = dir.path().join("unsub-plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(
            plugin_dir.join("init.lua"),
            r#"
                flag = false
                ps.sub("z", function(payload)
                    flag = true
                end)
                ps.unsub("z")
            "#,
        )
        .expect("write init.lua");
        registry.load_plugin(&plugin_dir).expect("load plugin");

        // When emitting the unsubscribed event.
        registry.emit("z", &serde_json::json!({}));

        // Then the callback was NOT invoked.
        let instances = registry.instances.borrow();
        let flag: bool = instances[0]
            .lua
            .globals()
            .get("flag")
            .expect("get flag");
        assert!(!flag, "callback should not have been invoked after unsub");
    }

    // ── ps.pub (plugin-internal) ──────────────────────────────────────

    #[rstest::rstest]
    fn ps_pub_fires_sub_callbacks_in_same_vm() {
        // Given a registry with a plugin that uses ps.pub/sub internally.
        let registry = test_registry();
        let dir = tempfile::tempdir().expect("create temp dir");
        let plugin_dir = dir.path().join("pub-plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(
            plugin_dir.join("init.lua"),
            r#"
                received = nil
                ps.sub("internal_evt", function(payload)
                    received = payload.msg
                end)
                ps.pub("internal_evt", { msg = "hello" })
            "#,
        )
        .expect("write init.lua");
        registry.load_plugin(&plugin_dir).expect("load plugin");

        // Then the internal pub/sub worked.
        let instances = registry.instances.borrow();
        let received: String = instances[0]
            .lua
            .globals()
            .get("received")
            .expect("get received");
        assert_eq!(received, "hello");
    }
}
