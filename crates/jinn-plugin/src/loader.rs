//! Plugin discovery and loading.
//!
//! Scans plugin directories for `init.lua` files. Each plugin directory
//! becomes a [`PluginMeta`]. Scripts are loaded into Lua states with
//! per-script `_ENV` isolation via `set_environment`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mlua::{Lua, RegistryKey};

use crate::sync_state::PluginHooks;

// ── Plugin Kinds ─────────────────────────────────────────────────────────

/// Whether a plugin is loaded globally at startup or attached per-session.
///
/// See `.plans/plugins-replace-workflows/plan.md` for the full design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluginKind {
    /// Always loaded into the shared Lua states at startup; fires for all sessions.
    Global,
    /// Loaded into a per-session Lua state when attached; fires only for that session.
    Attachable,
}

// ── Plugin Discovery ─────────────────────────────────────────────────────

/// Metadata for a discovered plugin.
#[derive(Debug, Clone)]
pub struct PluginMeta {
    /// Plugin name (directory name).
    pub name: String,
    /// Path to the plugin directory.
    pub path: PathBuf,
    /// Human-readable description from header comment.
    pub description: Option<String>,
    /// Whether this plugin is global (loaded at startup) or attachable (loaded per-session).
    pub kind: PluginKind,
}

/// Discover all plugins from user and system plugin directories.
///
/// Scans `system_dir/{global,attachable}/` and `user_dir/{global,attachable}/`.
/// User plugins override system plugins by name *within the same kind*. Global and
/// Attachable namespaces are independent: a global `foo` and an attachable `foo` are
/// both kept as distinct entries.
///
/// Falls back to a flat scan at the directory root (treating everything as Global)
/// when neither `global/` nor `attachable/` exists. This preserves back-compat with
/// the old single-directory layout.
///
/// Results sorted alphabetically by name.
pub fn discover_plugins(user_dir: &Path, system_dir: &Path) -> Vec<PluginMeta> {
    let mut seen: HashMap<(String, PluginKind), PluginMeta> = HashMap::new();

    // System plugins first (lower priority). Try new layout first, fall back to flat.
    for meta in scan_kind_dir(&system_dir.join("global"), PluginKind::Global) {
        seen.entry((meta.name.clone(), meta.kind)).or_insert(meta);
    }
    for meta in scan_kind_dir(&system_dir.join("attachable"), PluginKind::Attachable) {
        seen.entry((meta.name.clone(), meta.kind)).or_insert(meta);
    }
    if !system_dir.join("global").is_dir() && !system_dir.join("attachable").is_dir() {
        for meta in scan_kind_dir(system_dir, PluginKind::Global) {
            seen.entry((meta.name.clone(), meta.kind)).or_insert(meta);
        }
    }

    // User plugins override system within kind.
    for meta in scan_kind_dir(&user_dir.join("global"), PluginKind::Global) {
        seen.insert((meta.name.clone(), meta.kind), meta);
    }
    for meta in scan_kind_dir(&user_dir.join("attachable"), PluginKind::Attachable) {
        seen.insert((meta.name.clone(), meta.kind), meta);
    }
    if !user_dir.join("global").is_dir() && !user_dir.join("attachable").is_dir() {
        for meta in scan_kind_dir(user_dir, PluginKind::Global) {
            seen.insert((meta.name.clone(), meta.kind), meta);
        }
    }

    let mut plugins: Vec<PluginMeta> = seen.into_values().collect();
    plugins.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    plugins
}

/// Scan a single directory for plugin subdirectories containing `init.lua`.
///
/// All discovered plugins are tagged with the supplied `kind`.
fn scan_kind_dir(dir: &Path, kind: PluginKind) -> Vec<PluginMeta> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut plugins = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let init_lua = path.join("init.lua");
        if !init_lua.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_owned(),
            None => continue,
        };
        // Skip hidden directories.
        if name.starts_with('.') {
            continue;
        }
        let description = parse_description(&init_lua);
        plugins.push(PluginMeta {
            name,
            path,
            description,
            kind,
        });
    }
    plugins
}

/// Parse description from the first line of a Lua script.
///
/// Looks for `-- description: <text>` or `--- description: <text>`.
fn parse_description(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let first_line = content.lines().next()?;
    if first_line.is_empty() {
        return None;
    }

    let trimmed = first_line.trim_start();
    if let Some(rest) = trimmed
        .strip_prefix("--")
        .map(|s| s.strip_prefix('-').unwrap_or(s))
    {
        let rest = rest.trim();
        if let Some(desc) = rest.strip_prefix("description:") {
            let desc = desc.trim();
            if !desc.is_empty() {
                return Some(desc.to_owned());
            }
        }
    }
    None
}

// ── Plugin Loading ───────────────────────────────────────────────────────

/// Result of loading plugins into a Lua state.
///
/// Contains both hooks (for lifecycle events) and tool definitions (for LLM-callable functions).
pub struct LoadResult {
    /// Plugin name → hook data.
    pub hooks: HashMap<String, PluginHooks>,
    /// All tool definitions extracted from loaded plugins.
    pub tools: Vec<crate::tool_def::PluginToolDef>,
}

/// Load all plugins into a Lua state with `_ENV` isolation.
///
/// Each script is loaded with `set_environment` so globals from one plugin
/// are invisible to another. The returned table is stored in the Lua registry.
///
/// Returns a [`LoadResult`] containing both hooks and tool definitions.
pub fn load_all(lua: &Lua, plugins: &[PluginMeta]) -> LoadResult {
    let mut hooks = HashMap::new();
    let mut tools = Vec::new();

    for meta in plugins {
        let script_path = meta.path.join("init.lua");
        let source = match std::fs::read_to_string(&script_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(plugin = meta.name, err = %e, "failed to read plugin");
                continue;
            }
        };

        match load_plugin(lua, &source) {
            Ok(table_key) => {
                let table: mlua::Table = lua
                    .registry_value(&table_key)
                    .expect("just-stored key must resolve");
                let plugin_tools = crate::tool_def::extract_tools(lua, &table, &meta.name);
                tracing::debug!(
                    plugin = meta.name,
                    tools = plugin_tools.len(),
                    "loaded plugin"
                );
                tools.extend(plugin_tools);
                hooks.insert(meta.name.clone(), PluginHooks::new(table_key));
            }
            Err(e) => {
                tracing::error!(plugin = meta.name, err = %e, "failed to load plugin");
            }
        }
    }

    LoadResult { hooks, tools }
}

/// Load a single plugin script into the Lua state with `_ENV` isolation.
///
/// The script must return a table (the hook table). The returned table
/// is stored in the Lua registry.
fn load_plugin(lua: &Lua, source: &str) -> Result<RegistryKey, String> {
    // Create isolated environment for this script.
    let env = lua.create_table().map_err(|e| format!("create env: {e}"))?;

    // Expose the standard library via a metatable fallback. Writes still land
    // in `env` (per-plugin isolation preserved); reads of missing keys fall
    // through to `_G`, so `type`, `pairs`, `string.*`, etc. resolve.
    let metatable = lua.create_table().map_err(|e| format!("create mt: {e}"))?;
    metatable
        .set("__index", lua.globals())
        .map_err(|e| format!("set __index: {e}"))?;
    env.set_metatable(Some(metatable));

    // Load and evaluate with isolated _ENV.
    let result: mlua::Table = lua
        .load(source)
        .set_environment(env)
        .eval()
        .map_err(|e| format!("eval script: {e}"))?;

    // Store the returned table in the Lua registry.
    let key = lua
        .create_registry_value(result)
        .map_err(|e| format!("registry insert: {e}"))?;

    Ok(key)
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        reason = "test code"
    )]

    use super::*;

    fn make_plugin(dir: &Path, name: &str, init_content: &str) {
        let plugin_dir = dir.join(name);
        std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        std::fs::write(plugin_dir.join("init.lua"), init_content).expect("write init.lua");
    }

    // --- scan_dir ---

    #[test]
    fn scan_dir_finds_plugins_with_init_lua() {
        let dir = tempfile::tempdir().expect("tempdir");
        make_plugin(dir.path(), "alpha", "-- plugin alpha");

        let result = scan_kind_dir(dir.path(), PluginKind::Global);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "alpha");
        assert_eq!(result[0].kind, PluginKind::Global);
    }

    #[test]
    fn scan_dir_skips_dirs_without_init_lua() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("empty")).expect("create dir");

        let result = scan_kind_dir(dir.path(), PluginKind::Global);
        assert!(result.is_empty());
    }

    #[test]
    fn scan_dir_skips_hidden_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        make_plugin(dir.path(), ".hidden", "-- hidden");

        let result = scan_kind_dir(dir.path(), PluginKind::Global);
        assert!(result.is_empty());
    }

    // --- discover_plugins ---

    #[test]
    fn discover_plugins_user_overrides_system() {
        let user_dir = tempfile::tempdir().expect("tempdir");
        let system_dir = tempfile::tempdir().expect("tempdir");

        make_plugin(system_dir.path(), "shared", "-- description: System");
        make_plugin(user_dir.path(), "shared", "-- description: User");

        let result = discover_plugins(user_dir.path(), system_dir.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].description, Some("User".to_owned()));
    }

    #[test]
    fn discover_plugins_returns_sorted() {
        let user_dir = tempfile::tempdir().expect("tempdir");
        let system_dir = tempfile::tempdir().expect("tempdir");

        make_plugin(user_dir.path(), "charlie", "");
        make_plugin(user_dir.path(), "alpha", "");
        make_plugin(user_dir.path(), "bravo", "");

        let result = discover_plugins(user_dir.path(), system_dir.path());
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].name, "alpha");
        assert_eq!(result[1].name, "bravo");
        assert_eq!(result[2].name, "charlie");
    }

    #[test]
    fn discover_plugins_scans_global_and_attachable_subdirs() {
        let user_dir = tempfile::tempdir().expect("tempdir");
        let system_dir = tempfile::tempdir().expect("tempdir");

        // New layout: system_dir/{global,attachable}/<name>/init.lua
        make_plugin(&system_dir.path().join("global"), "welcome", "-- global");
        make_plugin(
            &system_dir.path().join("attachable"),
            "judge_fail",
            "-- attachable",
        );

        // Need parent dirs to exist before make_plugin writes into them.
        std::fs::create_dir_all(system_dir.path().join("global")).expect("mkdir global");
        std::fs::create_dir_all(system_dir.path().join("attachable")).expect("mkdir attachable");
        make_plugin(&system_dir.path().join("global"), "welcome", "-- global");
        make_plugin(
            &system_dir.path().join("attachable"),
            "judge_fail",
            "-- attachable",
        );

        let result = discover_plugins(user_dir.path(), system_dir.path());
        assert_eq!(result.len(), 2);
        let by_name: HashMap<&str, PluginKind> =
            result.iter().map(|m| (m.name.as_str(), m.kind)).collect();
        assert_eq!(by_name["welcome"], PluginKind::Global);
        assert_eq!(by_name["judge_fail"], PluginKind::Attachable);
    }

    #[test]
    fn discover_plugins_cross_kind_non_collision() {
        let user_dir = tempfile::tempdir().expect("tempdir");
        let system_dir = tempfile::tempdir().expect("tempdir");

        std::fs::create_dir_all(system_dir.path().join("global")).expect("mkdir");
        std::fs::create_dir_all(system_dir.path().join("attachable")).expect("mkdir");
        make_plugin(&system_dir.path().join("global"), "foo", "-- global foo");
        make_plugin(
            &system_dir.path().join("attachable"),
            "foo",
            "-- attachable foo",
        );

        let result = discover_plugins(user_dir.path(), system_dir.path());
        assert_eq!(result.len(), 2, "both foo entries must survive");
        let globals = result
            .iter()
            .filter(|m| m.kind == PluginKind::Global)
            .count();
        let attachables = result
            .iter()
            .filter(|m| m.kind == PluginKind::Attachable)
            .count();
        assert_eq!(globals, 1);
        assert_eq!(attachables, 1);
    }

    #[test]
    fn discover_plugins_user_overrides_system_within_kind() {
        let user_dir = tempfile::tempdir().expect("tempdir");
        let system_dir = tempfile::tempdir().expect("tempdir");

        std::fs::create_dir_all(system_dir.path().join("attachable")).expect("mkdir sys");
        std::fs::create_dir_all(user_dir.path().join("attachable")).expect("mkdir usr");
        make_plugin(
            &system_dir.path().join("attachable"),
            "judge",
            "-- description: SystemJudge",
        );
        make_plugin(
            &user_dir.path().join("attachable"),
            "judge",
            "-- description: UserJudge",
        );

        let result = discover_plugins(user_dir.path(), system_dir.path());
        let attachable: Vec<_> = result
            .iter()
            .filter(|m| m.kind == PluginKind::Attachable && m.name == "judge")
            .collect();
        assert_eq!(attachable.len(), 1);
        assert_eq!(attachable[0].description, Some("UserJudge".to_owned()));
    }

    // --- load_all / isolation ---

    #[test]
    fn load_all_isolates_plugin_globals() {
        let dir = tempfile::tempdir().expect("tempdir");
        make_plugin(
            dir.path(),
            "alpha",
            "x = 1\nreturn { on_test = function() return x end }",
        );
        make_plugin(
            dir.path(),
            "beta",
            "x = 2\nreturn { on_test = function() return x end }",
        );

        let plugins = discover_plugins(dir.path(), Path::new("/nonexistent"));
        let lua = Lua::new();
        let result = load_all(&lua, &plugins);

        assert_eq!(result.hooks.len(), 2);

        // Plugin alpha returns 1, plugin beta returns 2.
        for (name, ph) in &result.hooks {
            let table: mlua::Table = lua.registry_value(ph.table()).expect("get table");
            let func: mlua::Function = table.get("on_test").expect("get func");
            let result: i64 = func.call(()).expect("call");
            match name.as_str() {
                "alpha" => assert_eq!(result, 1),
                "beta" => assert_eq!(result, 2),
                _ => panic!("unknown plugin: {name}"),
            }
        }
    }

    #[test]
    fn load_plugin_exposes_stdlib_via_metatable() {
        let dir = tempfile::tempdir().expect("tempdir");
        make_plugin(
            &dir.path().join("global"),
            "stdlib",
            "return { on_test = function()
    if type(42) ~= 'number' then return 'bad type' end
    local count = 0
    for _k, _v in pairs({ a = 1, b = 2 }) do count = count + 1 end
    if count ~= 2 then return 'bad pairs' end
    local s = string.format('%d-%s', 7, 'x')
    if s ~= '7-x' then return 'bad string.format' end
    return 'ok'
end }",
        );

        let plugins = discover_plugins(dir.path(), Path::new("/nonexistent"));
        assert_eq!(
            plugins.len(),
            1,
            "stdlib plugin should be discovered: {plugins:?}"
        );

        let lua = Lua::new();
        let result = load_all(&lua, &plugins);
        assert_eq!(result.hooks.len(), 1, "stdlib plugin should load");

        let (_name, ph) = result.hooks.iter().next().expect("one hook");
        let table: mlua::Table = lua.registry_value(ph.table()).expect("get table");
        let func: mlua::Function = table.get("on_test").expect("get func");
        let result: String = func.call(()).expect("call");
        assert_eq!(result, "ok");
    }

    #[test]
    fn load_all_skips_syntax_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        make_plugin(dir.path(), "good", "return { on_test = function() end }");
        make_plugin(dir.path(), "bad_syntax", "this is not lua {{{");

        let plugins = discover_plugins(dir.path(), Path::new("/nonexistent"));
        let lua = Lua::new();
        let result = load_all(&lua, &plugins);

        // Only the good plugin should load.
        assert_eq!(result.hooks.len(), 1);
        assert!(result.hooks.contains_key("good"));
    }

    #[test]
    fn parse_description_extracts_from_header_comment() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("init.lua");
        std::fs::write(&file, "-- description: Hello world\nlocal x = 1").expect("write");

        let result = parse_description(&file);
        assert_eq!(result, Some("Hello world".to_owned()));
    }

    #[test]
    fn parse_description_returns_none_for_no_comment() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("init.lua");
        std::fs::write(&file, "local x = 1\n").expect("write");

        let result = parse_description(&file);
        assert!(result.is_none());
    }

    #[test]
    fn load_all_extracts_tool_definitions() {
        let dir = tempfile::tempdir().expect("tempdir");
        make_plugin(
            dir.path(),
            "tools_plugin",
            r#"
local M = {}
M.tools = {
    {
        name = "my_tool",
        description = "Does a thing",
        parameters = {
            { name = "msg", type = "string", description = "A message" },
        },
        handler = function(ctx, args)
            return "result: " .. args.msg
        end,
    },
}
function M.on_test() end
return M
"#,
        );

        let plugins = discover_plugins(dir.path(), Path::new("/nonexistent"));
        let lua = Lua::new();
        let result = load_all(&lua, &plugins);

        // Then the plugin has one tool definition.
        assert_eq!(result.tools.len(), 1);
        let tool = &result.tools[0];
        assert_eq!(tool.name, "my_tool");
        assert_eq!(tool.plugin_name, "tools_plugin");
        assert_eq!(tool.description, "Does a thing");
    }

    #[test]
    fn load_all_extracts_tool_with_no_parameters() {
        let dir = tempfile::tempdir().expect("tempdir");
        make_plugin(
            dir.path(),
            "simple_tool",
            r#"
local M = {}
M.tools = {
    {
        name = "no_params",
        description = "No params tool",
        parameters = {},
        handler = function(ctx)
            return "ok"
        end,
    },
}
function M.on_test() end
return M
"#,
        );

        let plugins = discover_plugins(dir.path(), Path::new("/nonexistent"));
        let lua = Lua::new();
        let result = load_all(&lua, &plugins);

        // Then the tool has an empty parameters schema.
        assert_eq!(result.tools.len(), 1);
        let tool = &result.tools[0];
        assert_eq!(tool.name, "no_params");
    }
}
