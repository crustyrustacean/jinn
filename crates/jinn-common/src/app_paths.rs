//! Application filesystem paths - single source of truth for all directory locations.
//!
//! [`AppPaths`] holds every base directory the application needs. It is computed
//! once at init time: [`AppPaths::default()`] for production (from `dirs::*`,
//! honoring the [`PATHS_ROOT_ENV`] override), [`AppPaths::new_in()`] for tests
//! (from a temp root).
//!
//! All domain code that needs a filesystem path must read it from the injected
//! `AppPaths` instance - never from `dirs::*` free functions directly.

use crate::app_info::APP_NAME;
use marked_path::{Absolute, MarkedPath, MarkedPathAccess, MarkedPathBuf};
use std::path::PathBuf;

/// Browser profile mode: one persistent profile per mode.
///
/// `headless` and `headed` each get their own profile subdir so a single
/// warmed headed profile can be shared by both `web-fetch` and `web-search`
/// without colliding on Chromium's singleton lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserProfileMode {
    /// Headless profile (no visible window).
    Headless,
    /// Headed profile (visible window, manually warmable).
    Headed,
}

impl BrowserProfileMode {
    /// Returns the subdirectory name for this mode.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Headless => "headless",
            Self::Headed => "headed",
        }
    }
}

/// Application filesystem paths.
///
/// Stores app-specific directories (not platform directories). For production,
/// these are derived from `dirs::*` — or from a single root directory when
/// [`PATHS_ROOT_ENV`] is set (see [`AppPaths::default()`]). For tests, from a
/// single temp root via [`AppPaths::new_in`].
///
/// Construct once at init and share via `Services.paths`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(clippy::struct_field_names, reason = "names match domain terminology")]
pub struct AppPaths {
    /// `~/.config/jinn` - providers.toml, prompts/, personas/, themes/, jinn.toml
    config_dir: MarkedPathBuf<Absolute>,
    /// `~/.local/share/jinn` - sessions.db
    data_dir: MarkedPathBuf<Absolute>,
    /// `~/.cache/jinn` - model_cache.json
    cache_dir: MarkedPathBuf<Absolute>,
    /// `~/.local/state/jinn` - jinn.log
    ///
    /// Falls back to `data_dir` on platforms where `dirs::state_dir()` returns
    /// `None` (e.g. macOS, Windows).
    state_dir: MarkedPathBuf<Absolute>,
    /// `~/` - home directory (skills live at `~/.agents/skills`)
    home_dir: MarkedPathBuf<Absolute>,
    /// `/usr/share/jinn` - system-wide defaults (themes, personas, prompts).
    ///
    /// User files in `~/.config/jinn/` override system files of the same name.
    system_data_dir: MarkedPathBuf<Absolute>,
}

/// Environment variable that, when set, rebinds every [`AppPaths`] base
/// under one root directory.
///
/// Read once, at construction of [`AppPaths::default()`]; no other code in
/// the workspace reads the environment for path derivation. When set, the
/// layout matches [`AppPaths::new_in`] exactly: `config/`, `data/`,
/// `cache/`, `state/` subdirectories, `home` is the root itself, and
/// system data lives at `root/share`.
///
/// Its purpose is test isolation: integration tests that spawn the `jinn`
/// binary point this at a temp root so no run ever writes to the real
/// user config, data, or home directories. On Linux, where `dirs`
/// already honors `XDG_CONFIG_HOME`/`XDG_DATA_HOME`, the override wins
/// there too, keeping one behavior across platforms.
pub const PATHS_ROOT_ENV: &str = "JINN_PATHS_ROOT";

/// The system-wide data directory of the platform layout.
///
/// The platform layout installs jinn's shipped defaults (themes, personas,
/// prompts, skills) here; the [`PATHS_ROOT_ENV`] override layout puts them
/// under the override root instead. Named once so the production default
/// and the tests that assert it name the same location.
const SYSTEM_DATA_DIR: &str = "/usr/share/jinn";

impl Default for AppPaths {
    // The env override is a test hook read once at startup; a malformed value
    // is a programming error in the harness, not runtime state.
    #![allow(
        clippy::expect_used,
        reason = "test-hook env var, malformed value is a harness bug"
    )]
    fn default() -> Self {
        match std::env::var_os(PATHS_ROOT_ENV) {
            Some(root) => {
                let typed_root = MarkedPathBuf::<Absolute>::new(&root)
                    .expect("PATHS_ROOT_ENV override root is absolute");
                Self::new_in(&typed_root.as_marked_path())
            }
            None => Self::from_platform_dirs(),
        }
    }
}

/// The current working directory in absolute form.
///
/// The last-resort base when the platform cannot name a directory. If even
/// the working directory cannot be read (it was deleted), the temp directory
/// stands in; both candidates are absolute by construction.
///
/// # Panics
///
/// Panics if the candidate directory is not absolute, which cannot happen on
/// supported platforms (the OS returns absolute cwds and temp paths).
#[expect(
    clippy::expect_used,
    reason = "platform-impossible: OS cwds and temp paths are absolute"
)]
fn cwd_absolute() -> MarkedPathBuf<Absolute> {
    let candidate = std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir());
    MarkedPathBuf::<Absolute>::new(candidate).expect("cwd and temp dir are absolute")
}

impl AppPaths {
    // Bare-name joins onto bases already typed `Absolute` cannot introduce a
    // drive prefix or absolute segment, so those `join`/`join_all` results are
    // infallible by construction; the panics guard programmer error, not
    // runtime state, and the invariant is documented here once for the family.
    #![allow(
        clippy::expect_used,
        clippy::missing_panics_doc,
        reason = "bare-name joins on typed bases are infallible by construction"
    )]

    /// Creates paths derived from the platform's per-user directories.
    ///
    /// Every base is absolute: when the platform cannot name a directory,
    /// the current working directory stands in, resolved to its absolute
    /// form so no base is ever relative.
    ///
    /// # Panics
    ///
    /// Panics if a platform directory is not absolute, which cannot happen
    /// on supported platforms.
    fn from_platform_dirs() -> Self {
        let fallback = cwd_absolute();
        let typed = |raw: Option<PathBuf>| {
            raw.map_or_else(
                || fallback.clone(),
                |raw| {
                    MarkedPathBuf::<Absolute>::new(raw).expect("platform dirs and cwd are absolute")
                },
            )
        };
        Self {
            config_dir: typed(dirs::config_dir()),
            data_dir: typed(dirs::data_dir()),
            cache_dir: typed(dirs::cache_dir()),
            state_dir: typed(dirs::state_dir().or_else(dirs::data_dir)),
            home_dir: typed(dirs::home_dir()),
            system_data_dir: MarkedPathBuf::<Absolute>::new(SYSTEM_DATA_DIR)
                .expect("SYSTEM_DATA_DIR constant is absolute"),
        }
    }

    /// Creates paths derived from a single root directory (for tests).
    ///
    /// All subdirectories are nested under `root/` so that a single
    /// `TempDir` cleans everything up.
    #[must_use]
    pub fn new_in(root: &MarkedPath<'_, Absolute>) -> Self {
        // One `Result` for the whole fixed layout: every dir is a bare
        // name join onto the validated root.
        let dir = |name| {
            root.join_all([name])
                .expect("joining a name keeps the root absolute")
        };
        Self {
            config_dir: dir("config"),
            data_dir: dir("data"),
            cache_dir: dir("cache"),
            state_dir: dir("state"),
            home_dir: root.to_owned(),
            system_data_dir: dir("share"),
        }
    }

    /// Sets the home directory for tests (bounds the project walk).
    #[cfg(feature = "testing")]
    pub fn set_home_dir_for_test(&mut self, home: MarkedPathBuf<Absolute>) {
        self.home_dir = home;
    }

    // -- Derived paths -------------------------------------------------------

    /// Session database parent directory (`~/.local/share/jinn`).
    ///
    /// Pass to `SqliteSessionStore::new_in()`.
    #[must_use]
    pub fn sessions_dir(&self) -> MarkedPathBuf<Absolute> {
        self.data_dir
            .join(APP_NAME)
            .expect("name join keeps the base absolute")
    }

    /// Parent directory for builtin graph databases (`~/.local/share/jinn/graph`).
    ///
    /// The builtin overgraph MCP server opens one database directory per
    /// configured server entry beneath this dir (default: `graph/default`).
    #[must_use]
    pub fn graph_db_dir(&self) -> MarkedPathBuf<Absolute> {
        self.data_dir
            .join_all([APP_NAME, "graph"])
            .expect("name joins keep the base absolute")
    }

    /// Base directory for persistent browser profiles (`~/.local/share/jinn/browser-profile`).
    ///
    /// Per-mode profiles live under subdirs (`browser-profile/headless`,
    /// `browser-profile/headed`) so a single warmed headed profile is shared by
    /// both `web-fetch` and `web-search` when they select `headed-chrome`.
    /// Use [`Self::browser_profile_dir_for_mode`] to resolve a concrete path.
    #[must_use]
    pub fn browser_profile_base_dir(&self) -> MarkedPathBuf<Absolute> {
        self.data_dir
            .join_all([APP_NAME, "browser-profile"])
            .expect("name joins keep the base absolute")
    }

    /// Persistent browser profile for a given mode (`headless` or `headed`).
    #[must_use]
    pub fn browser_profile_dir_for_mode(
        &self,
        mode: BrowserProfileMode,
    ) -> MarkedPathBuf<Absolute> {
        self.browser_profile_base_dir()
            .join(mode.as_str())
            .expect("name join keeps the base absolute")
    }

    /// Prompt templates directory (`~/.config/jinn/prompts`).
    #[must_use]
    pub fn prompts_dir(&self) -> MarkedPathBuf<Absolute> {
        self.config_dir
            .join_all([APP_NAME, "prompts"])
            .expect("name joins keep the base absolute")
    }

    /// Agent skills directory (`~/.agents/skills`).
    #[must_use]
    pub fn skills_dir(&self) -> MarkedPathBuf<Absolute> {
        self.home_dir
            .join_all([".agents", "skills"])
            .expect("name joins keep the base absolute")
    }

    /// User home directory (`~/`). Bounds the project walk (exclusive of home).
    #[must_use]
    pub fn home_dir(&self) -> &MarkedPathBuf<Absolute> {
        &self.home_dir
    }

    /// Personas directory (`~/.config/jinn/personas`).
    #[must_use]
    pub fn personas_dir(&self) -> MarkedPathBuf<Absolute> {
        self.config_dir
            .join_all([APP_NAME, "personas"])
            .expect("name joins keep the base absolute")
    }

    /// Workflows directory (`~/.config/jinn/workflows`).
    ///
    /// One directory per workflow, each holding a `workflow.md`. Launch-global:
    /// unlike skills and prompts, workflows are not project-scoped, so the
    /// discovery scan reads this one directory regardless of session cwd.
    #[must_use]
    pub fn workflows_dir(&self) -> MarkedPathBuf<Absolute> {
        self.config_dir
            .join_all([APP_NAME, "workflows"])
            .expect("name joins keep the base absolute")
    }

    /// Model cache file (`~/.cache/jinn/model_cache.json`).
    #[must_use]
    pub fn cache_path(&self) -> MarkedPathBuf<Absolute> {
        self.cache_dir
            .join_all([APP_NAME, "model_cache.json"])
            .expect("name joins keep the base absolute")
    }

    /// Provider config file (`~/.config/jinn/providers.toml`).
    #[must_use]
    pub fn config_path(&self) -> MarkedPathBuf<Absolute> {
        self.config_dir
            .join_all([APP_NAME, "providers.toml"])
            .expect("name joins keep the base absolute")
    }

    /// Log file path (`~/.local/state/jinn/jinn.log` on Linux).
    #[must_use]
    pub fn log_path(&self) -> MarkedPathBuf<Absolute> {
        self.state_dir
            .join(APP_NAME)
            .and_then(|j| j.join(format!("{APP_NAME}.log")))
            .expect("name joins keep the base absolute")
    }

    /// App state file (`~/.local/state/jinn/state.toml` on Linux).
    #[must_use]
    pub fn state_file_path(&self) -> MarkedPathBuf<Absolute> {
        self.state_dir
            .join_all([APP_NAME, "state.toml"])
            .expect("name joins keep the base absolute")
    }

    /// Themes directory (`~/.config/jinn/themes`).
    #[must_use]
    pub fn themes_dir(&self) -> MarkedPathBuf<Absolute> {
        self.config_dir
            .join_all([APP_NAME, "themes"])
            .expect("name joins keep the base absolute")
    }

    /// User preferences file (`~/.config/jinn/jinn.toml`).
    #[must_use]
    pub fn preferences_path(&self) -> MarkedPathBuf<Absolute> {
        self.config_dir
            .join_all([APP_NAME, crate::app_info::PREFS_FILE_NAME])
            .expect("name joins keep the base absolute")
    }

    // -- System data directories (XDG fallback) -----------------------------

    /// System data directory (`/usr/share/jinn`).
    ///
    /// Used as a fallback source for themes, personas, and prompts
    /// when user-specific files are not found.
    #[must_use]
    pub fn system_data_dir(&self) -> &MarkedPathBuf<Absolute> {
        &self.system_data_dir
    }

    /// Config directory (`~/.config/jinn`).
    #[must_use]
    pub fn app_config_dir(&self) -> MarkedPathBuf<Absolute> {
        self.config_dir
            .join(APP_NAME)
            .expect("name join keeps the base absolute")
    }

    /// Base config directory (`~/.config`) — parent of all jinn config.
    #[must_use]
    pub fn config_dir(&self) -> &MarkedPathBuf<Absolute> {
        &self.config_dir
    }

    /// Base data directory (`~/.local/share`) — parent of all jinn data.
    #[must_use]
    pub fn data_dir(&self) -> &MarkedPathBuf<Absolute> {
        &self.data_dir
    }

    /// App data directory (`~/.local/share/jinn`) — caches, scratch
    /// dirs, and other jinn-owned data live here.
    #[must_use]
    pub fn app_data_dir(&self) -> MarkedPathBuf<Absolute> {
        self.data_dir
            .join(APP_NAME)
            .expect("name join keeps the base absolute")
    }

    /// System themes directory (`/usr/share/jinn/themes`).
    #[must_use]
    pub fn system_themes_dir(&self) -> MarkedPathBuf<Absolute> {
        self.system_data_dir
            .join("themes")
            .expect("name join keeps the base absolute")
    }

    /// System prompts directory (`/usr/share/jinn/prompts`).
    #[must_use]
    pub fn system_prompts_dir(&self) -> MarkedPathBuf<Absolute> {
        self.system_data_dir
            .join("prompts")
            .expect("name join keeps the base absolute")
    }
    /// System skills directory (`/usr/share/jinn/skills`).
    ///
    /// Installed by the system package. Used as a fallback source for
    /// skills when user-global skills are not found.
    #[must_use]
    pub fn system_skills_dir(&self) -> MarkedPathBuf<Absolute> {
        self.system_data_dir
            .join("skills")
            .expect("name join keeps the base absolute")
    }

    /// User's models.dev reference file (`~/.cache/jinn/models.dev.json`).
    ///
    /// Written by `jinn fetch models`. Contains the full models.dev API
    /// dump used as a fallback for context window lookup.
    #[must_use]
    pub fn models_dev_user_path(&self) -> MarkedPathBuf<Absolute> {
        self.cache_dir
            .join_all([APP_NAME, "models.dev.json"])
            .expect("name joins keep the base absolute")
    }

    /// System models.dev reference file (`/usr/share/jinn/models.dev.json`).
    ///
    /// Installed by the system package. Used as a fallback when no
    /// user-level file exists.
    #[must_use]
    pub fn models_dev_system_path(&self) -> MarkedPathBuf<Absolute> {
        self.system_data_dir
            .join("models.dev.json")
            .expect("name join keeps the base absolute")
    }

    /// Epoch benchmark cache directory (`~/.cache/jinn/epoch`).
    ///
    /// Holds the three CSVs `<c-r>` extracts from the downloaded epoch
    /// zip; absent on first run, where the compiled-in seed stands in.
    #[must_use]
    pub fn epoch_cache_dir(&self) -> MarkedPathBuf<Absolute> {
        self.cache_dir
            .join_all([APP_NAME, "epoch"])
            .expect("name joins keep the base absolute")
    }

    /// OpenRouter market cache directory (`~/.cache/jinn/openrouter`).
    ///
    /// Holds the model-market's TTL disk caches: the models listing and
    /// the per-model endpoint detail files.
    #[must_use]
    pub fn openrouter_cache_dir(&self) -> MarkedPathBuf<Absolute> {
        self.cache_dir
            .join_all([APP_NAME, "openrouter"])
            .expect("name joins keep the base absolute")
    }

    /// Embedding-model cache directory (`~/.cache/jinn/embed-models`).
    ///
    /// Holds the overgraph server's local embedding model weights,
    /// downloaded from HuggingFace on first use and reused offline after
    /// that. Re-downloadable content, so it lives under the cache dir.
    #[must_use]
    pub fn embed_models_dir(&self) -> MarkedPathBuf<Absolute> {
        self.cache_dir
            .join_all([APP_NAME, "embed-models"])
            .expect("name joins keep the base absolute")
    }

    // -- Merged resource paths (system + user) ---------------------------------

    /// Returns merged theme file paths from system and user directories.
    ///
    /// System themes are listed first. If a user theme has the same filename
    /// as a system theme, the user version replaces the system one in the result.
    /// The returned list is sorted by filename stem.
    #[must_use]
    pub fn resolve_theme_paths(&self) -> Vec<(String, MarkedPathBuf<Absolute>)> {
        resolve_resource_paths(&self.system_themes_dir(), &self.themes_dir(), "toml")
    }

    /// Returns merged prompt template file paths from system and user directories.
    ///
    /// Same merge semantics as [`resolve_theme_paths`].
    #[must_use]
    pub fn resolve_prompt_paths(&self) -> Vec<(String, MarkedPathBuf<Absolute>)> {
        resolve_resource_paths(&self.system_prompts_dir(), &self.prompts_dir(), "md")
    }
}

/// Scans two directories for files with the given extension, merging by filename stem.
///
/// System files are listed first. User files with the same stem replace system ones.
/// Results are sorted by stem.
fn resolve_resource_paths(
    system_dir: &MarkedPathBuf<Absolute>,
    user_dir: &MarkedPathBuf<Absolute>,
    extension: &str,
) -> Vec<(String, MarkedPathBuf<Absolute>)> {
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
    map: &mut std::collections::BTreeMap<String, MarkedPathBuf<Absolute>>,
    dir: &MarkedPathBuf<Absolute>,
    extension: &str,
) {
    let Ok(entries) = std::fs::read_dir(dir.as_path()) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(typed) = MarkedPathBuf::<Absolute>::new(&path) else {
            continue;
        };
        if typed.extension().is_some_and(|ext| ext == extension)
            && let Some(name) = typed.file_stem().and_then(|s| s.to_str())
        {
            map.insert(name.to_owned(), typed);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;

    /// Serializes every test that mutates [`PATHS_ROOT_ENV`], and restores the
    /// prior value on drop so one test's override never leaks into another.
    ///
    /// The lock must be held before the env is touched (edition 2024 makes
    /// `set_var`/`remove_var` unsafe because the environment is process-global).
    static PATHS_ROOT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Restores [`PATHS_ROOT_ENV`] to `value`, clearing it when `None`.
    ///
    /// Must only be called while `PATHS_ROOT_LOCK` is held.
    fn restore_paths_root_env(value: Option<&std::ffi::OsStr>) {
        match value {
            Some(value) => {
                // SAFETY: PATHS_ROOT_LOCK is held by the caller, so no other
                // test mutates or observes the environment mid-restore.
                unsafe { std::env::set_var(PATHS_ROOT_ENV, value) };
            }
            None => {
                // SAFETY: PATHS_ROOT_LOCK is held by the caller (see above).
                unsafe { std::env::remove_var(PATHS_ROOT_ENV) };
            }
        }
    }

    /// Guard that sets [`PATHS_ROOT_ENV`] and restores the prior value on drop.
    struct PathsRootEnvGuard {
        /// Held for the lifetime of the guard, serializing env mutation.
        _lock: std::sync::MutexGuard<'static, ()>,
        /// The value to restore (None clears the variable).
        previous: Option<std::ffi::OsString>,
    }

    impl PathsRootEnvGuard {
        /// Sets `JINN_PATHS_ROOT` to `root`, remembering what was there.
        fn set(root: &MarkedPathBuf<Absolute>) -> Self {
            let lock = PATHS_ROOT_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let previous = std::env::var_os(PATHS_ROOT_ENV);
            // SAFETY: the lock is held before any env mutation, so no other
            // test mutates or observes the environment concurrently.
            unsafe { std::env::set_var(PATHS_ROOT_ENV, root.as_path()) };
            Self {
                _lock: lock,
                previous,
            }
        }
    }

    impl Drop for PathsRootEnvGuard {
        fn drop(&mut self) {
            // The lock guard outlives this body (field drop order), so the
            // restore stays serialized.
            restore_paths_root_env(self.previous.as_deref());
        }
    }

    #[rstest::rstest]
    fn default_honors_paths_root_env_for_every_base() {
        // Given a set JINN_PATHS_ROOT.
        let root = jinn_testutil::tempdir();
        let _guard = PathsRootEnvGuard::set(&root.path);

        // When constructing the default AppPaths.
        let paths = AppPaths::default();

        // Then every base rebinds under the root with the new_in layout:
        // config/data/cache/state subdirs, home as the root, share for
        // system data.
        assert_eq!(
            paths.config_dir(),
            &root.path.join("config").expect("root joins a bare name")
        );
        assert_eq!(
            paths.data_dir(),
            &root.path.join("data").expect("root joins a bare name")
        );
        let cache_parent = root
            .path
            .join_all(["cache", "jinn"])
            .expect("root joins bare names");
        assert_eq!(
            paths.cache_path().parent(),
            Some(cache_parent.as_marked_path())
        );
        let state_parent = root
            .path
            .join_all(["state", "jinn"])
            .expect("root joins bare names");
        assert_eq!(
            paths.state_file_path().parent(),
            Some(state_parent.as_marked_path())
        );
        assert_eq!(paths.home_dir(), &root.path);
        assert_eq!(
            paths.system_data_dir(),
            &root.path.join("share").expect("root joins a bare name")
        );
    }

    #[rstest::rstest]
    fn default_with_paths_root_env_matches_new_in() {
        // Given a set JINN_PATHS_ROOT.
        let root = jinn_testutil::tempdir();
        let _guard = PathsRootEnvGuard::set(&root.path);

        // When constructing the default and a new_in AppPaths from the
        // same root.
        let via_env = AppPaths::default();
        let via_new_in = AppPaths::new_in(&root.path.as_marked_path());

        // Then they are identical — the override is exactly the new_in
        // layout, not a variant of it.
        assert_eq!(via_env, via_new_in);
    }

    #[rstest::rstest]
    fn default_without_paths_root_env_uses_platform_dirs() {
        // Given no JINN_PATHS_ROOT in the environment (set then clear, both
        // under the lock, so the test is hermetic against ambient env).
        let root = jinn_testutil::tempdir();
        let _guard = PathsRootEnvGuard::set(&root.path);
        let _cleared = ClearPathsRootGuard::new();

        // When constructing the default AppPaths.
        let paths = AppPaths::default();

        // Then the system data dir keeps its platform default (the marker
        // that distinguishes the platform layout from the override layout).
        assert_eq!(
            paths.system_data_dir(),
            &MarkedPathBuf::<Absolute>::new(SYSTEM_DATA_DIR)
                .expect("SYSTEM_DATA_DIR constant is absolute")
        );
    }

    /// Guard that clears [`PATHS_ROOT_ENV`] for its lifetime, restoring on
    /// drop. The PATHS_ROOT lock must already be held by the caller.
    struct ClearPathsRootGuard {
        /// The value to restore (None leaves the variable cleared).
        previous: Option<std::ffi::OsString>,
    }

    impl ClearPathsRootGuard {
        fn new() -> Self {
            let previous = std::env::var_os(PATHS_ROOT_ENV);
            // SAFETY: the caller holds PATHS_ROOT_LOCK across this guard's
            // lifetime, serializing env mutation against other tests.
            unsafe { std::env::remove_var(PATHS_ROOT_ENV) };
            Self { previous }
        }
    }

    impl Drop for ClearPathsRootGuard {
        fn drop(&mut self) {
            // The outer PathsRootEnvGuard's lock is still held here.
            restore_paths_root_env(self.previous.as_deref());
        }
    }

    #[rstest::rstest]
    fn workflows_dir_sits_beside_personas_under_config() {
        // Given an AppPaths rooted at a temp tree.
        let root = jinn_testutil::tempdir();
        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When resolving the workflows dir.
        let dir = paths.workflows_dir();

        // Then it is the config dir's workflows folder, beside personas.
        assert_eq!(
            dir,
            root.path
                .join_all(["config", "jinn", "workflows"])
                .expect("root joins bare names")
        );
        // And it is a sibling of the personas dir.
        assert_eq!(dir.parent(), paths.personas_dir().parent());
    }

    #[rstest::rstest]
    fn resolve_theme_paths_returns_empty_when_both_dirs_missing() {
        // Given an AppPaths with nonexistent system and user dirs.
        let root = jinn_testutil::tempdir();
        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When resolving theme paths.
        let result = paths.resolve_theme_paths();

        // Then the result is empty.
        assert_eq!(
            result,
            [] as [(std::string::String, MarkedPathBuf<Absolute>); 0]
        );
    }

    #[rstest::rstest]
    fn resolve_theme_paths_finds_system_files() {
        // Given an AppPaths with a system themes dir containing one theme.
        let root = jinn_testutil::tempdir();
        let system_dir = root
            .path
            .join("share/themes")
            .expect("temp root joins a bare name");
        std::fs::create_dir_all(&system_dir).expect("create dir");
        std::fs::write(
            system_dir
                .join("ocean.toml")
                .expect("typed parent joins a bare name"),
            "focus_accent = \"blue\"",
        )
        .expect("write");

        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When resolving theme paths.
        let result = paths.resolve_theme_paths();

        // Then the system theme is found.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "ocean");
    }

    #[rstest::rstest]
    fn resolve_theme_paths_finds_user_files() {
        // Given an AppPaths with a user themes dir containing one theme.
        let root = jinn_testutil::tempdir();
        let user_dir = root
            .path
            .join("config/jinn/themes")
            .expect("temp root joins a bare name");
        std::fs::create_dir_all(&user_dir).expect("create dir");
        std::fs::write(
            user_dir
                .join("forest.toml")
                .expect("typed parent joins a bare name"),
            "focus_accent = \"green\"",
        )
        .expect("write");

        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When resolving theme paths.
        let result = paths.resolve_theme_paths();

        // Then the user theme is found.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "forest");
    }

    #[rstest::rstest]
    fn resolve_theme_paths_user_overrides_system() {
        // Given system and user dirs both with a theme named "ocean".
        let root = jinn_testutil::tempdir();

        let system_dir = root
            .path
            .join("share/themes")
            .expect("temp root joins a bare name");
        std::fs::create_dir_all(&system_dir).expect("create dir");
        std::fs::write(
            system_dir
                .join("ocean.toml")
                .expect("typed parent joins a bare name"),
            "focus_accent = \"blue\"",
        )
        .expect("write");

        let user_dir = root
            .path
            .join("config/jinn/themes")
            .expect("temp root joins a bare name");
        std::fs::create_dir_all(&user_dir).expect("create dir");
        std::fs::write(
            user_dir
                .join("ocean.toml")
                .expect("typed parent joins a bare name"),
            "focus_accent = \"red\"",
        )
        .expect("write");

        let paths = AppPaths::new_in(&root.path.as_marked_path());

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
        let root = jinn_testutil::tempdir();

        let system_dir = root
            .path
            .join("share/themes")
            .expect("temp root joins a bare name");
        std::fs::create_dir_all(&system_dir).expect("create dir");
        std::fs::write(
            system_dir
                .join("ocean.toml")
                .expect("typed parent joins a bare name"),
            "focus_accent = \"blue\"",
        )
        .expect("write");

        let user_dir = root
            .path
            .join("config/jinn/themes")
            .expect("temp root joins a bare name");
        std::fs::create_dir_all(&user_dir).expect("create dir");
        std::fs::write(
            user_dir
                .join("forest.toml")
                .expect("typed parent joins a bare name"),
            "focus_accent = \"green\"",
        )
        .expect("write");

        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When resolving theme paths.
        let result = paths.resolve_theme_paths();

        // Then both themes are found, sorted by name.
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "forest");
        assert_eq!(result[1].0, "ocean");
    }

    #[rstest::rstest]
    fn resolve_prompt_paths_uses_md_extension() {
        // Given a system prompts dir with a .md file.
        let root = jinn_testutil::tempdir();
        let system_dir = root
            .path
            .join("share/prompts")
            .expect("temp root joins a bare name");
        std::fs::create_dir_all(&system_dir).expect("create dir");
        std::fs::write(
            system_dir
                .join("example.md")
                .expect("typed parent joins a bare name"),
            "content",
        )
        .expect("write");

        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When resolving prompt paths.
        let result = paths.resolve_prompt_paths();

        // Then the .md file is found.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "example");
    }

    #[rstest::rstest]
    fn default_system_data_dir_is_usr_share() {
        // Given a default AppPaths, constructed with no ambient override —
        // the lock is held so a parallel test's transient JINN_PATHS_ROOT
        // cannot leak into this construction.
        let root = jinn_testutil::tempdir();
        let _guard = PathsRootEnvGuard::set(&root.path);
        let _cleared = ClearPathsRootGuard::new();
        let paths = AppPaths::default();

        // When reading the system data dir.
        let system_data_dir = paths.system_data_dir();

        // Then it is /usr/share/jinn.
        assert_eq!(
            system_data_dir,
            &MarkedPathBuf::<Absolute>::new(SYSTEM_DATA_DIR)
                .expect("SYSTEM_DATA_DIR constant is absolute")
        );
    }

    #[rstest::rstest]
    fn new_in_system_data_dir_is_under_root() {
        // Given an AppPaths created with new_in.
        let root = jinn_testutil::tempdir();
        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When reading the system data dir.
        let system_data_dir = paths.system_data_dir();

        // Then it is root/share.
        assert_eq!(
            system_data_dir,
            &root.path.join("share").expect("root joins a bare name")
        );
    }

    #[rstest::rstest]
    fn new_in_epoch_cache_dir_is_under_root_cache() {
        // Given an AppPaths created with new_in.
        let root = jinn_testutil::tempdir();
        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When reading the epoch cache dir.
        let epoch_dir = paths.epoch_cache_dir();

        // Then it is root/cache/jinn/epoch.
        assert_eq!(
            epoch_dir,
            root.path
                .join_all(["cache", "jinn"])
                .expect("root joins bare names")
                .join("epoch")
                .expect("typed parent joins a bare name"),
        );
    }

    #[rstest::rstest]
    fn new_in_openrouter_cache_dir_is_under_root_cache() {
        // Given an AppPaths created with new_in.
        let root = jinn_testutil::tempdir();
        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When reading the OpenRouter market cache dir.
        let or_dir = paths.openrouter_cache_dir();

        // Then it is root/cache/jinn/openrouter.
        assert_eq!(
            or_dir,
            root.path
                .join_all(["cache", "jinn"])
                .expect("root joins bare names")
                .join("openrouter")
                .expect("typed parent joins a bare name"),
        );
    }

    #[rstest::rstest]
    fn default_system_skills_dir_is_usr_share() {
        // Given a default AppPaths, constructed with no ambient override —
        // the lock is held so a parallel test's transient JINN_PATHS_ROOT
        // cannot leak into this construction.
        let root = jinn_testutil::tempdir();
        let _guard = PathsRootEnvGuard::set(&root.path);
        let _cleared = ClearPathsRootGuard::new();
        let paths = AppPaths::default();

        // When reading the system skills dir.
        let system_skills_dir = paths.system_skills_dir();

        // Then it is /usr/share/jinn/skills.
        assert_eq!(
            system_skills_dir,
            MarkedPathBuf::<Absolute>::new(SYSTEM_DATA_DIR)
                .expect("SYSTEM_DATA_DIR constant is absolute")
                .join("skills")
                .expect("typed parent joins a bare name")
        );
    }

    #[rstest::rstest]
    fn new_in_embed_models_dir_is_under_root_cache() {
        // Given an AppPaths created with new_in.
        let root = jinn_testutil::tempdir();
        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When reading the embedding-model cache dir.
        let dir = paths.embed_models_dir();

        // Then it is root/cache/jinn/embed-models.
        assert_eq!(
            dir,
            root.path
                .join_all(["cache", "jinn"])
                .expect("root joins bare names")
                .join("embed-models")
                .expect("typed parent joins a bare name"),
        );
    }

    #[rstest::rstest]
    fn new_in_system_skills_dir_is_under_root() {
        // Given an AppPaths created with new_in.
        let root = jinn_testutil::tempdir();
        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When reading the system skills dir.
        let system_skills_dir = paths.system_skills_dir();

        // Then it is root/share/skills.
        assert_eq!(
            system_skills_dir,
            root.path
                .join("share/skills")
                .expect("temp root joins a bare name")
        );
    }

    #[rstest::rstest]
    fn log_path_under_state_dir_in_new_in() {
        // Given an AppPaths rooted in a temp dir.
        let root = jinn_testutil::tempdir();
        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When reading the log path.
        let log_path = paths.log_path();

        // Then it is root/state/jinn/jinn.log.
        assert_eq!(
            log_path,
            root.path
                .join_all(["state", "jinn"])
                .expect("root joins bare names")
                .join("jinn.log")
                .expect("typed parent joins a bare name"),
        );
    }

    #[rstest::rstest]
    fn state_file_path_under_state_dir_in_new_in() {
        // Given an AppPaths rooted in a temp dir.
        let root = jinn_testutil::tempdir();
        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When reading the state file path.
        let state_file_path = paths.state_file_path();

        // Then it is root/state/jinn/state.toml.
        assert_eq!(
            state_file_path,
            root.path
                .join_all(["state", "jinn"])
                .expect("root joins bare names")
                .join("state.toml")
                .expect("typed parent joins a bare name"),
        );
    }

    #[rstest::rstest]
    fn log_path_filename_is_app_name_dot_log() {
        // Given an AppPaths.
        let root = jinn_testutil::tempdir();
        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When reading the filename component of the log path.
        let log_path = paths.log_path();
        let filename = log_path.file_name().and_then(|s| s.to_str());

        // Then it is jinn.log.
        assert_eq!(filename, Some("jinn.log"));
    }

    #[rstest::rstest]
    fn browser_profile_dir_for_mode_lands_under_data_base() {
        // Given an AppPaths rooted at a temp dir.
        let root = jinn_testutil::tempdir();
        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When resolving the headed profile dir.
        let headed = paths.browser_profile_dir_for_mode(BrowserProfileMode::Headed);
        let headless = paths.browser_profile_dir_for_mode(BrowserProfileMode::Headless);

        // Then both land under data/jinn/browser-profile/<mode>.
        assert_eq!(
            headed,
            root.path
                .join("data/jinn/browser-profile/headed")
                .expect("temp root joins a bare name")
        );
        assert_eq!(
            headless,
            root.path
                .join("data/jinn/browser-profile/headless")
                .expect("temp root joins a bare name")
        );
    }

    #[rstest::rstest]
    fn graph_db_dir_lands_under_data_base() {
        // Given an AppPaths rooted at a temp dir.
        let root = jinn_testutil::tempdir();
        let paths = AppPaths::new_in(&root.path.as_marked_path());

        // When resolving the graph database dir.
        let dir = paths.graph_db_dir();

        // Then it is data/jinn/graph.
        assert_eq!(
            dir,
            root.path
                .join("data/jinn/graph")
                .expect("temp root joins a bare name")
        );
    }
}
