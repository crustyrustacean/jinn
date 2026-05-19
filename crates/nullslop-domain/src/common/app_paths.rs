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
#[allow(clippy::struct_field_names)]
pub struct AppPaths {
    /// `~/.config/nullslop` — providers.toml, prompts/, personas/, plugins/, themes/, nullslop.toml
    config_dir: PathBuf,
    /// `~/.local/share/nullslop` — sessions.db
    data_dir: PathBuf,
    /// `~/.cache/nullslop` — model_cache.json
    cache_dir: PathBuf,
    /// `~/` — home directory (skills live at `~/.agents/skills`)
    home_dir: PathBuf,
    /// `/usr/share/nullslop` — system-wide defaults (themes, personas, prompts).
    ///
    /// User files in `~/.config/nullslop/` override system files of the same name.
    system_data_dir: PathBuf,
}

impl Default for AppPaths {
    fn default() -> Self {
        Self {
            config_dir: dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")),
            data_dir: dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")),
            cache_dir: dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".")),
            home_dir: dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")),
            system_data_dir: PathBuf::from("/usr/share/nullslop"),
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
            system_data_dir: root.join("share"),
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


    // -- System data directories (XDG fallback) -----------------------------

    /// System data directory (`/usr/share/nullslop`).
    ///
    /// Used as a fallback source for themes, personas, and prompts
    /// when user-specific files are not found.
    #[must_use]
    pub fn system_data_dir(&self) -> &Path {
        &self.system_data_dir
    }

    /// System themes directory (`/usr/share/nullslop/themes`).
    #[must_use]
    pub fn system_themes_dir(&self) -> PathBuf {
        self.system_data_dir.join("themes")
    }

    /// System personas directory (`/usr/share/nullslop/personas`).
    #[must_use]
    pub fn system_personas_dir(&self) -> PathBuf {
        self.system_data_dir.join("personas")
    }

    /// System prompts directory (`/usr/share/nullslop/prompts`).
    #[must_use]
    pub fn system_prompts_dir(&self) -> PathBuf {
        self.system_data_dir.join("prompts")
    }

    // -- Merged resource paths (system + user) ---------------------------------

    /// Returns merged theme file paths from system and user directories.
    ///
    /// System themes are listed first. If a user theme has the same filename
    /// as a system theme, the user version replaces the system one in the result.
    /// The returned list is sorted by filename stem.
    #[must_use]
    pub fn resolve_theme_paths(&self) -> Vec<(String, PathBuf)> {
        resolve_resource_paths(&self.system_themes_dir(), &self.themes_dir(), "toml")
    }

    /// Returns merged persona file paths from system and user directories.
    ///
    /// Same merge semantics as [`resolve_theme_paths`].
    #[must_use]
    pub fn resolve_persona_paths(&self) -> Vec<(String, PathBuf)> {
        resolve_resource_paths(&self.system_personas_dir(), &self.personas_dir(), "md")
    }

    /// Returns merged prompt template file paths from system and user directories.
    ///
    /// Same merge semantics as [`resolve_theme_paths`].
    #[must_use]
    pub fn resolve_prompt_paths(&self) -> Vec<(String, PathBuf)> {
        resolve_resource_paths(&self.system_prompts_dir(), &self.prompts_dir(), "md")
    }
}

/// Scans two directories for files with the given extension, merging by filename stem.
///
/// System files are listed first. User files with the same stem replace system ones.
/// Results are sorted by stem.
fn resolve_resource_paths(
    system_dir: &Path,
    user_dir: &Path,
    extension: &str,
) -> Vec<(String, PathBuf)> {
    use std::collections::BTreeMap;

    let mut map = BTreeMap::new();

    // System files first (lower priority).
    scan_dir_into(&mut map, system_dir, extension);
    // User files override system files of the same name.
    scan_dir_into(&mut map, user_dir, extension);

    map.into_iter().collect()
}

/// Scans a directory for files with the given extension and inserts into the map by stem.
fn scan_dir_into(
    map: &mut std::collections::BTreeMap<String, PathBuf>,
    dir: &Path,
    extension: &str,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == extension)
            && let Some(name) = path.file_stem().and_then(|s| s.to_str())
        {
            map.insert(name.to_owned(), path);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    #[rstest::rstest]
    fn resolve_theme_paths_returns_empty_when_both_dirs_missing() {
        // Given an AppPaths with nonexistent system and user dirs.
        let root = tempfile::TempDir::new().expect("temp dir");
        let paths = AppPaths::new_in(root.path());

        // When resolving theme paths.
        let result = paths.resolve_theme_paths();

        // Then the result is empty.
        assert!(result.is_empty());
    }

    #[rstest::rstest]
    fn resolve_theme_paths_finds_system_files() {
        // Given an AppPaths with a system themes dir containing one theme.
        let root = tempfile::TempDir::new().expect("temp dir");
        let system_dir = root.path().join("share/themes");
        std::fs::create_dir_all(&system_dir).expect("create dir");
        std::fs::write(system_dir.join("ocean.toml"), "focus_accent = \"blue\"").expect("write");

        let paths = AppPaths::new_in(root.path());

        // When resolving theme paths.
        let result = paths.resolve_theme_paths();

        // Then the system theme is found.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "ocean");
    }

    #[rstest::rstest]
    fn resolve_theme_paths_finds_user_files() {
        // Given an AppPaths with a user themes dir containing one theme.
        let root = tempfile::TempDir::new().expect("temp dir");
        let user_dir = root.path().join("config/nullslop/themes");
        std::fs::create_dir_all(&user_dir).expect("create dir");
        std::fs::write(user_dir.join("forest.toml"), "focus_accent = \"green\"").expect("write");

        let paths = AppPaths::new_in(root.path());

        // When resolving theme paths.
        let result = paths.resolve_theme_paths();

        // Then the user theme is found.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "forest");
    }

    #[rstest::rstest]
    fn resolve_theme_paths_user_overrides_system() {
        // Given system and user dirs both with a theme named "ocean".
        let root = tempfile::TempDir::new().expect("temp dir");

        let system_dir = root.path().join("share/themes");
        std::fs::create_dir_all(&system_dir).expect("create dir");
        std::fs::write(system_dir.join("ocean.toml"), "focus_accent = \"blue\"").expect("write");

        let user_dir = root.path().join("config/nullslop/themes");
        std::fs::create_dir_all(&user_dir).expect("create dir");
        std::fs::write(user_dir.join("ocean.toml"), "focus_accent = \"red\"").expect("write");

        let paths = AppPaths::new_in(root.path());

        // When resolving theme paths.
        let result = paths.resolve_theme_paths();

        // Then only one "ocean" entry exists (user version).
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "ocean");
        // And it points to the user directory.
        assert!(result[0].1.starts_with(user_dir));
    }

    #[rstest::rstest]
    fn resolve_theme_paths_merges_both_dirs() {
        // Given system has "ocean", user has "forest".
        let root = tempfile::TempDir::new().expect("temp dir");

        let system_dir = root.path().join("share/themes");
        std::fs::create_dir_all(&system_dir).expect("create dir");
        std::fs::write(system_dir.join("ocean.toml"), "focus_accent = \"blue\"").expect("write");

        let user_dir = root.path().join("config/nullslop/themes");
        std::fs::create_dir_all(&user_dir).expect("create dir");
        std::fs::write(user_dir.join("forest.toml"), "focus_accent = \"green\"").expect("write");

        let paths = AppPaths::new_in(root.path());

        // When resolving theme paths.
        let result = paths.resolve_theme_paths();

        // Then both themes are found, sorted by name.
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "forest");
        assert_eq!(result[1].0, "ocean");
    }

    #[rstest::rstest]
    fn resolve_persona_paths_uses_md_extension() {
        // Given a system personas dir with a .md file and a .txt file.
        let root = tempfile::TempDir::new().expect("temp dir");
        let system_dir = root.path().join("share/personas");
        std::fs::create_dir_all(&system_dir).expect("create dir");
        std::fs::write(system_dir.join("coder.md"), "content").expect("write");
        std::fs::write(system_dir.join("notes.txt"), "content").expect("write");

        let paths = AppPaths::new_in(root.path());

        // When resolving persona paths.
        let result = paths.resolve_persona_paths();

        // Then only .md files are found.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "coder");
    }

    #[rstest::rstest]
    fn resolve_prompt_paths_uses_md_extension() {
        // Given a system prompts dir with a .md file.
        let root = tempfile::TempDir::new().expect("temp dir");
        let system_dir = root.path().join("share/prompts");
        std::fs::create_dir_all(&system_dir).expect("create dir");
        std::fs::write(system_dir.join("example.md"), "content").expect("write");

        let paths = AppPaths::new_in(root.path());

        // When resolving prompt paths.
        let result = paths.resolve_prompt_paths();

        // Then the .md file is found.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "example");
    }

    #[rstest::rstest]
    fn default_system_data_dir_is_usr_share() {
        // Given a default AppPaths.
        let paths = AppPaths::default();

        // Then the system data dir is /usr/share/nullslop.
        assert_eq!(paths.system_data_dir(), Path::new("/usr/share/nullslop"));
    }

    #[rstest::rstest]
    fn new_in_system_data_dir_is_under_root() {
        // Given an AppPaths created with new_in.
        let root = tempfile::TempDir::new().expect("temp dir");
        let paths = AppPaths::new_in(root.path());

        // Then the system data dir is root/share.
        assert_eq!(paths.system_data_dir(), root.path().join("share"));
    }
}
