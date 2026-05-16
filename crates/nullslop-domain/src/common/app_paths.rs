//! Application filesystem paths — single source of truth for all directory locations.
//!
//! [`AppPaths`] holds every base directory the application needs. It is computed
//! once at init time: [`AppPaths::default()`] for production (from `dirs::*`),
//! [`AppPaths::new_in()`] for tests (from a temp root).
//!
//! All domain code that needs a filesystem path must read it from the injected
//! `AppPaths` instance — never from `dirs::*` free functions directly.

use std::path::{Path, PathBuf};

use super::app_info::APP_NAME;

/// Application filesystem paths.
///
/// Stores app-specific directories (not platform directories). For production,
/// these are derived from `dirs::*`. For tests, from a single temp root.
///
/// Construct once at init and share via `Services.paths`.
#[derive(Debug, Clone)]
pub struct AppPaths {
    /// `~/.config/nullslop` — providers.toml, prompts/, personas/, plugins/, themes/, nullslop.toml
    config_dir: PathBuf,
    /// `~/.local/share/nullslop` — sessions.db
    data_dir: PathBuf,
    /// `~/.cache/nullslop` — model_cache.json
    cache_dir: PathBuf,
    /// `~/` — home directory (skills live at `~/.agents/skills`)
    home_dir: PathBuf,
}

impl Default for AppPaths {
    fn default() -> Self {
        Self {
            config_dir: dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")),
            data_dir: dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")),
            cache_dir: dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".")),
            home_dir: dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")),
        }
    }
}

impl AppPaths {
    /// Creates paths derived from a single root directory (for tests).
    ///
    /// All subdirectories are nested under `root/` so that a single
    /// `TempDir` cleans everything up.
    #[must_use]
    pub fn new_in(root: &Path) -> Self {
        Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
            home_dir: root.to_path_buf(),
        }
    }

    // -- Derived paths -------------------------------------------------------

    /// Session database parent directory (`~/.local/share/nullslop`).
    ///
    /// Pass to `SqliteSessionStore::new_in()`.
    #[must_use]
    pub fn sessions_dir(&self) -> PathBuf {
        self.data_dir.join(APP_NAME)
    }

    /// Prompt templates directory (`~/.config/nullslop/prompts`).
    #[must_use]
    pub fn prompts_dir(&self) -> PathBuf {
        self.config_dir.join(APP_NAME).join("prompts")
    }

    /// Agent skills directory (`~/.agents/skills`).
    #[must_use]
    pub fn skills_dir(&self) -> PathBuf {
        self.home_dir.join(".agents").join("skills")
    }

    /// Personas directory (`~/.config/nullslop/personas`).
    #[must_use]
    pub fn personas_dir(&self) -> PathBuf {
        self.config_dir.join(APP_NAME).join("personas")
    }

    /// Model cache file (`~/.cache/nullslop/model_cache.json`).
    #[must_use]
    pub fn cache_path(&self) -> PathBuf {
        self.cache_dir.join(APP_NAME).join("model_cache.json")
    }

    /// Provider config file (`~/.config/nullslop/providers.toml`).
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.config_dir.join(APP_NAME).join("providers.toml")
    }

    /// Themes directory (`~/.config/nullslop/themes`).
    #[must_use]
    pub fn themes_dir(&self) -> PathBuf {
        self.config_dir.join(APP_NAME).join("themes")
    }

    /// User preferences file (`~/.config/nullslop/nullslop.toml`).
    #[must_use]
    pub fn preferences_path(&self) -> PathBuf {
        self.config_dir
            .join(APP_NAME)
            .join(super::app_info::PREFS_FILE_NAME)
    }

    /// Plugins directory (`~/.config/nullslop/plugins`).
    #[must_use]
    pub fn plugins_dir(&self) -> PathBuf {
        self.config_dir.join(APP_NAME).join("plugins")
    }
}
