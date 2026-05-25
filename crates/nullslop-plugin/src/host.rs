//! Plugin host — owns the Lua VM and orchestrates plugin loading and dispatch.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use error_stack::Report;
use mlua::Lua;
use wherror::Error;

use crate::bindings;
use crate::loader;

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

/// Metadata about a loaded plugin.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    /// The plugin directory name (e.g., `"welcome"`).
    pub name: String,
    /// The path to the plugin directory.
    pub path: PathBuf,
}

/// Errors from plugin loading.
#[derive(Debug, Error)]
#[error(debug)]
pub struct PluginError;

/// The plugin host — owns the Lua VM, loads plugins, and dispatches events.
pub struct PluginHost {
    /// The Lua virtual machine.
    lua: Lua,
    /// The command sender callback.
    #[expect(dead_code, reason = "needed for future wiring phases")]
    sender: CommandSender,
}

impl PluginHost {
    /// Creates a new plugin host.
    ///
    /// Initializes the Lua VM, installs the `ns` and `ps` bindings, and
    /// prepares the preflight hook map.
    ///
    /// # Errors
    ///
    /// Returns an error if Lua binding installation fails.
    pub fn new(sender: CommandSender) -> Result<Self, Report<PluginError>> {
        let lua = Lua::new();

        bindings::install(&lua, &sender).map_err(|e| {
            tracing::error!(err = %e, "failed to install Lua bindings");
            Report::new(PluginError).attach("Lua binding installation failed")
        })?;

        crate::preflight::init(&lua);

        Ok(Self { lua, sender })
    }

    /// Loads a single plugin from a directory containing `init.lua`.
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

        let source = std::fs::read_to_string(&init_path).map_err(|e| {
            tracing::error!(err = %e, path = %init_path.display(), "failed to read init.lua");
            Report::new(PluginError).attach(format!("read error: {}", init_path.display()))
        })?;

        self.lua
            .load(&source)
            .set_name(format!("plugin/{name}/init.lua"))
            .exec()
            .map_err(|e| {
                tracing::error!(err = %e, plugin = %name, "plugin init.lua failed");
                Report::new(PluginError).attach(format!("plugin '{name}' failed to load"))
            })?;

        tracing::info!(plugin = %name, "loaded plugin");

        Ok(PluginInfo {
            name,
            path: dir.to_path_buf(),
        })
    }

    /// Loads all plugins from a directory.
    ///
    /// Scans for subdirectories containing `init.lua`, loads each one, and
    /// logs warnings for any that fail. Returns info for successfully loaded
    /// plugins.
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

    /// Dispatches an event to all Lua subscribers registered via `ps.sub`.
    ///
    /// Calls each subscriber's callback with the JSON payload converted to
    /// a Lua table. Errors in individual callbacks are logged, not propagated.
    pub fn dispatch_event(&self, event_name: &str, payload: &serde_json::Value) {
        let lua = &self.lua;

        // Get the subscription map from app_data.
        let Some(guard) = lua.app_data_ref::<bindings::SubscriptionMap>() else {
            return;
        };

        let callbacks: Vec<mlua::Function> = {
            let map = guard.lock();
            map.get(event_name).cloned().unwrap_or_default()
        };

        if callbacks.is_empty() {
            return;
        }

        // Convert JSON payload to Lua value.
        let lua_payload = match crate::bindings::json_to_lua_value(lua, payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(err = %e, "failed to convert payload to Lua");
                return;
            }
        };

        for callback in callbacks {
            if let Err(e) = callback.call::<()>(lua_payload.clone()) {
                tracing::warn!(err = %e, event_name, "subscriber callback error");
            }
        }
    }

    /// Runs preflight hooks for a command name.
    ///
    /// Returns `true` if all hooks approve (or no hooks are registered).
    /// Returns `false` if any hook vetoes.
    pub fn dispatch_preflight(&self, command_name: &str, payload: &serde_json::Value) -> bool {
        crate::preflight::dispatch(&self.lua, command_name, payload)
    }

    /// Returns a reference to the Lua VM.
    ///
    /// Useful for tests that need to inspect Lua state.
    #[must_use]
    pub fn lua(&self) -> &Lua {
        &self.lua
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

    #[rstest::rstest]
    fn new_creates_host_with_ns_and_ps_globals() {
        // Given a command sender.
        let (sender, _) = test_sender();

        // When creating a plugin host.
        let host = PluginHost::new(sender).expect("host creation");

        // Then the Lua VM has ns and ps globals.
        let globals = host.lua().globals();
        let ns: mlua::Table = globals.get("ns").expect("ns table exists");
        let ps: mlua::Table = globals.get("ps").expect("ps table exists");
        assert!(ns.get::<mlua::Function>("emit").is_ok());
        assert!(ps.get::<mlua::Function>("sub").is_ok());
        assert!(ps.get::<mlua::Function>("pub").is_ok());
        assert!(ps.get::<mlua::Function>("unsub").is_ok());
    }

    #[rstest::rstest]
    fn load_plugin_from_valid_dir_succeeds() {
        // Given a temp directory with a valid init.lua.
        let (sender, _) = test_sender();
        let host = PluginHost::new(sender).expect("host creation");

        let dir = tempfile::tempdir().expect("create temp dir");
        let plugin_dir = dir.path().join("test-plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(plugin_dir.join("init.lua"), "-- test plugin").expect("write init.lua");

        // When loading the plugin.
        let result = host.load_plugin(&plugin_dir);

        // Then it succeeds with correct info.
        let info = result.expect("load should succeed");
        assert_eq!(info.name, "test-plugin");
    }

    #[rstest::rstest]
    fn load_plugin_from_missing_dir_returns_error() {
        // Given a nonexistent path.
        let (sender, _) = test_sender();
        let host = PluginHost::new(sender).expect("host creation");

        // When loading from a nonexistent directory.
        let result = host.load_plugin(Path::new("/nonexistent/path"));

        // Then it returns an error.
        assert!(result.is_err(), "should fail for missing directory");
    }

    #[rstest::rstest]
    fn load_plugin_from_dir_without_init_returns_error() {
        // Given a directory without init.lua.
        let (sender, _) = test_sender();
        let host = PluginHost::new(sender).expect("host creation");

        let dir = tempfile::tempdir().expect("create temp dir");

        // When loading from a directory without init.lua.
        let result = host.load_plugin(dir.path());

        // Then it returns an error.
        assert!(result.is_err(), "should fail without init.lua");
    }

    #[rstest::rstest]
    fn load_all_loads_multiple_plugins() {
        // Given a plugins directory with 3 valid plugins.
        let (sender, _) = test_sender();
        let host = PluginHost::new(sender).expect("host creation");

        let dir = tempfile::tempdir().expect("create temp dir");
        for name in ["alpha", "beta", "gamma"] {
            let plugin_dir = dir.path().join(name);
            fs::create_dir_all(&plugin_dir).expect("create dir");
            fs::write(plugin_dir.join("init.lua"), format!("-- {name}")).expect("write init.lua");
        }

        // When loading all plugins.
        let loaded = host.load_all(dir.path());

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
        let (sender, _) = test_sender();
        let host = PluginHost::new(sender).expect("host creation");

        let dir = tempfile::tempdir().expect("create temp dir");

        // Valid plugins.
        for name in ["valid1", "valid2"] {
            let plugin_dir = dir.path().join(name);
            fs::create_dir_all(&plugin_dir).expect("create dir");
            fs::write(plugin_dir.join("init.lua"), format!("-- {name}")).expect("write init.lua");
        }

        // Invalid: dir with no init.lua.
        let invalid = dir.path().join("broken");
        fs::create_dir_all(&invalid).expect("create dir");

        // When loading all plugins.
        let loaded = host.load_all(dir.path());

        // Then 2 plugins are loaded, no panic.
        assert_eq!(loaded.len(), 2);
    }
}
