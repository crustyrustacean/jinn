//! Plugin discovery — scans plugin directories and extracts metadata from header comments.
//!
//! Called once at startup to populate [`AppState::discovered_plugins`](crate::common::app_state::AppState::discovered_plugins).
//! Two search paths (user takes priority over system):
//!
//! ```text
//! ~/.config/jinn/plugins/<name>/init.lua     (user plugins)
//! /usr/share/jinn/plugins/<name>/init.lua    (system plugins)
//! ```

use std::collections::HashMap;
use std::path::Path;

use crate::common::app_paths::AppPaths;

/// Metadata for a discovered Lua plugin.
#[derive(Debug, Clone)]
pub struct PluginMeta {
    /// Plugin name (directory name).
    pub name: String,
    /// Human-readable description extracted from the first-line header comment.
    pub description: Option<String>,
}

/// Discovers all Lua plugins from user and system plugin directories.
///
/// Scans both directories, deduplicates (user overrides system by name),
/// and returns results sorted alphabetically by name.
pub fn discover_plugins(paths: &AppPaths) -> Vec<PluginMeta> {
    let mut seen: HashMap<String, PluginMeta> = HashMap::new();

    // System plugins first (lower priority).
    for meta in scan_dir(&paths.system_plugins_dir()) {
        seen.entry(meta.name.clone()).or_insert(meta);
    }

    // User plugins override system.
    for meta in scan_dir(&paths.plugins_dir()) {
        seen.insert(meta.name.clone(), meta);
    }

    let mut plugins: Vec<PluginMeta> = seen.into_values().collect();
    plugins.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    plugins
}

/// Scans a single directory for plugin subdirectories containing `init.lua`.
fn scan_dir(dir: &Path) -> Vec<PluginMeta> {
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
        let description = parse_description(&init_lua);
        plugins.push(PluginMeta { name, description });
    }
    plugins
}

/// Parses the description from the first line of a Lua script.
///
/// Looks for a header comment in the form `-- description: <text>`.
/// Returns `None` if the first line doesn't match the convention.
fn parse_description(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let first_line = content.lines().next()?;
    if first_line.is_empty() {
        return None;
    }

    let trimmed = first_line.trim_start();
    // Match `-- description:` or `--- description:`
    if let Some(rest) = trimmed
        .strip_prefix("--")
        .and_then(|s| Some(s.strip_prefix('-').unwrap_or(s)))
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn make_plugin(dir: &Path, name: &str, init_content: &str) -> PathBuf {
        let plugin_dir = dir.join(name);
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(plugin_dir.join("init.lua"), init_content).expect("write init.lua");
        plugin_dir
    }

    // --- parse_description ---

    #[rstest::rstest]
    fn parse_description_extracts_from_header_comment() {
        // Given a temp file with a description header.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("init.lua");
        fs::write(&file, "-- description: Hello world\nlocal x = 1").expect("write");

        // When parsing.
        let result = parse_description(&file);

        // Then it extracts the description.
        assert_eq!(result, Some("Hello world".to_owned()));
    }

    #[rstest::rstest]
    fn parse_description_extracts_from_triple_dash_comment() {
        // Given a temp file with a triple-dash description header.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("init.lua");
        fs::write(&file, "--- description: LuaDoc style\nlocal x = 1").expect("write");

        // When parsing.
        let result = parse_description(&file);

        // Then it extracts the description.
        assert_eq!(result, Some("LuaDoc style".to_owned()));
    }

    #[rstest::rstest]
    fn parse_description_returns_none_for_no_comment() {
        // Given a temp file without a description comment.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("init.lua");
        fs::write(&file, "local x = 1\n").expect("write");

        // When parsing.
        let result = parse_description(&file);

        // Then it returns None.
        assert!(result.is_none());
    }

    #[rstest::rstest]
    fn parse_description_returns_none_for_empty_file() {
        // Given an empty file.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("init.lua");
        fs::write(&file, "").expect("write");

        // When parsing.
        let result = parse_description(&file);

        // Then it returns None.
        assert!(result.is_none());
    }

    #[rstest::rstest]
    fn parse_description_returns_none_for_empty_description() {
        // Given a file with an empty description field.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("init.lua");
        fs::write(&file, "-- description:\nlocal x = 1").expect("write");

        // When parsing.
        let result = parse_description(&file);

        // Then it returns None (empty description is not useful).
        assert!(result.is_none());
    }

    // --- scan_dir ---

    #[rstest::rstest]
    fn scan_dir_finds_plugins_with_init_lua() {
        // Given a directory with a valid plugin.
        let dir = tempfile::tempdir().expect("tempdir");
        make_plugin(dir.path(), "my_plugin", "-- description: Test plugin");

        // When scanning.
        let result = scan_dir(dir.path());

        // Then it finds the plugin.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "my_plugin");
        assert_eq!(result[0].description, Some("Test plugin".to_owned()));
    }

    #[rstest::rstest]
    fn scan_dir_skips_dirs_without_init_lua() {
        // Given a directory with a subdirectory missing init.lua.
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("empty_dir")).expect("create dir");

        // When scanning.
        let result = scan_dir(dir.path());

        // Then it finds nothing.
        assert!(result.is_empty());
    }

    #[rstest::rstest]
    fn scan_dir_returns_empty_for_nonexistent_dir() {
        // Given a nonexistent directory.
        let result = scan_dir(Path::new("/nonexistent/path"));

        // Then it returns empty.
        assert!(result.is_empty());
    }

    // --- discover_plugins ---

    #[rstest::rstest]
    fn discover_plugins_finds_user_plugins() {
        // Given a user plugin directory with a plugin.
        // AppPaths::new_in(root) => plugins_dir() = root/config/jinn/plugins
        let root = tempfile::tempdir().expect("tempdir");
        let user_plugins_dir = root.path().join("config").join("jinn").join("plugins");
        fs::create_dir_all(&user_plugins_dir).expect("create user plugin dir");
        make_plugin(&user_plugins_dir, "hello", "-- description: Says hello");

        let paths = AppPaths::new_in(root.path());

        // When discovering.
        let result = discover_plugins(&paths);

        // Then it finds the plugin.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "hello");
    }

    #[rstest::rstest]
    fn discover_plugins_user_overrides_system() {
        // Given same-named plugin in both user and system dirs.
        let root = tempfile::tempdir().expect("tempdir");
        let user_plugins_dir = root.path().join("config").join("jinn").join("plugins");
        let system_plugins_dir = root.path().join("share").join("plugins");
        fs::create_dir_all(&user_plugins_dir).expect("create user dir");
        fs::create_dir_all(&system_plugins_dir).expect("create system dir");
        make_plugin(
            &system_plugins_dir,
            "shared",
            "-- description: System version",
        );
        make_plugin(&user_plugins_dir, "shared", "-- description: User version");

        let paths = AppPaths::new_in(root.path());

        // When discovering.
        let result = discover_plugins(&paths);

        // Then the user version wins.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].description, Some("User version".to_owned()));
    }

    #[rstest::rstest]
    fn discover_plugins_returns_sorted() {
        // Given multiple plugins.
        let root = tempfile::tempdir().expect("tempdir");
        let user_plugins_dir = root.path().join("config").join("jinn").join("plugins");
        fs::create_dir_all(&user_plugins_dir).expect("create user dir");
        make_plugin(&user_plugins_dir, "charlie", "");
        make_plugin(&user_plugins_dir, "alpha", "");
        make_plugin(&user_plugins_dir, "bravo", "");

        let paths = AppPaths::new_in(root.path());

        // When discovering.
        let result = discover_plugins(&paths);

        // Then they are sorted.
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].name, "alpha");
        assert_eq!(result[1].name, "bravo");
        assert_eq!(result[2].name, "charlie");
    }
}
