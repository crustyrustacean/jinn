//! Plugin loader — discovers `main.rhai` files in the plugins directory.

use std::path::{Path, PathBuf};

use crate::app_info::APP_NAME;
use crate::PluginId;

/// Returns the default plugins directory (`~/.config/nullslop/plugins/`).
///
/// # Panics
///
/// Panics if the system config directory cannot be determined.
#[must_use]
pub fn plugins_dir() -> PathBuf {
    dirs::config_dir()
        .expect("cannot determine config directory")
        .join(APP_NAME)
        .join("plugins")
}

/// A discovered plugin on disk.
pub struct DiscoveredPlugin {
    /// The plugin's directory name (used as PluginId).
    pub id: PluginId,
    /// Path to `main.rhai`.
    pub path: PathBuf,
}

/// Scans the plugins directory for subdirectories containing `main.rhai`.
pub fn discover(plugins_dir: &Path) -> Vec<DiscoveredPlugin> {
    let dir = match std::fs::read_dir(plugins_dir) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    dir.filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }
            let main_rhai = path.join("main.rhai");
            if !main_rhai.exists() {
                return None;
            }
            let name = path.file_name()?.to_str()?.to_owned();
            Some(DiscoveredPlugin {
                id: PluginId::new(&name),
                path: main_rhai,
            })
        })
        .collect()
}
