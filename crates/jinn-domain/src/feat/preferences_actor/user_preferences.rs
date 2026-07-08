//! User preferences data type and file I/O.
//!
//! Defines [`UserPreferences`] as the schema for `jinn.toml`,
//! along with loading and saving logic. The file lives at
//! `~/.config/jinn/jinn.toml` and is auto-created on first run from
//! [`DEFAULT_CONFIG`] (a comment-rich template embedded at compile time).

use std::path::{Path, PathBuf};

use crate::common::app_info::{APP_NAME, PREFS_FILE_NAME};
use crate::common::toml_patch::DocumentPatcher;
use error_stack::{Report, ResultExt as _};
use serde::{Deserialize, Serialize};
use wherror::Error;

// ── Re-exports: configs co-located with their consuming features ───────
// The structs below have been moved out of this file to their natural feature
// homes. They are re-exported here so existing consumer import paths
// (`crate::feat::preferences_actor::user_preferences::*`) keep resolving.
pub use crate::feat::auto_prune_worker::AutoPruneConfig;
pub use crate::feat::auto_prune_worker::anchor_shield::AnchorShieldConfig;
pub use crate::feat::auto_prune_worker::anchored_assistant::AnchoredAssistantAutoPruneConfig;
pub use crate::feat::auto_prune_worker::broken_edit::BrokenEditAutoPruneConfig;
pub use crate::feat::auto_prune_worker::consecutive_reads::ConsecutiveReadsAutoPruneConfig;
pub use crate::feat::auto_prune_worker::double_edit::DoubleEditAutoPruneConfig;
pub use crate::feat::auto_prune_worker::edit_read::EditReadAutoPruneConfig;
pub use crate::feat::auto_prune_worker::read_edit::ReadEditAutoPruneConfig;
pub use crate::feat::auto_prune_worker::regex::{RegexAutoPruneConfig, RegexPruneRule};
pub use crate::feat::auto_prune_worker::todo_prune::TodoAutoPruneConfig;
pub use crate::feat::auto_prune_worker::tool_age_window::ToolAgeWindowAutoPruneConfig;
pub use crate::feat::auto_prune_worker::trivial_assistant::TrivialAssistantAutoPruneConfig;
pub use crate::feat::compaction_worker::CompactionConfig;
pub use crate::feat::cwd_input::CwdSelectorConfig;
pub use crate::feat::llm_actor::RequestRetryConfig;
pub use crate::feat::project::ProjectConfig;
pub use crate::feat::session_lifecycle::SessionLifecycle;
pub use crate::feat::tools_actor::OpenrouterWebSearchConfig;
pub use crate::feat::ui::MinimapConfig;
pub use crate::feat::web_fetch_actor::{WebFetchBackend, WebFetchConfig};
pub use crate::feat::web_search_actor::WebSearchConfig;

/// Canonical default `jinn.toml` embedded at compile time.
///
/// Used both to auto-create the file on first run and to back the
/// `jinn config init` subcommand. A round-trip equality test in this
/// module's test suite asserts that this string deserializes to
/// exactly `UserPreferences::default()`, which is the CI gate that
/// prevents the shipped template from drifting from the struct.
pub(crate) const DEFAULT_CONFIG: &str = include_str!("default_jinn.toml");

/// Default seconds without a chat-history change before a session is
/// considered hung. Covers HTTP handshake hangs, keepalive-only connections,
/// and stalled tool batches — anything that stops mutating the visible history.
pub(crate) const DEFAULT_HISTORY_STALL_TIMEOUT_SECS: u64 = 60;

/// Default maximum stall retries before the watchdog gives up and cancels the turn.
pub(crate) const DEFAULT_STALL_RETRY_MAX_RETRIES: u32 = 3;

/// Default base delay (seconds) for stall-retry exponential backoff. Tighter
/// than `[request_retry]` base_delay because the stall window itself provides
/// the bulk of the wait between attempts.
pub(crate) const DEFAULT_STALL_RETRY_BASE_DELAY_SECS: u64 = 2;

/// Default maximum cap (seconds) for stall-retry exponential backoff.
pub(crate) const DEFAULT_STALL_RETRY_MAX_DELAY_SECS: u64 = 30;

/// Serde default function for [`UserPreferences::history_stall_timeout_secs`].
pub(crate) fn default_history_stall_timeout_secs() -> u64 {
    DEFAULT_HISTORY_STALL_TIMEOUT_SECS
}

/// Serde default function for [`UserPreferences::stall_retry_max_retries`].
pub(crate) fn default_stall_retry_max_retries() -> u32 {
    DEFAULT_STALL_RETRY_MAX_RETRIES
}

/// Serde default function for [`UserPreferences::stall_retry_base_delay_secs`].
pub(crate) fn default_stall_retry_base_delay_secs() -> u64 {
    DEFAULT_STALL_RETRY_BASE_DELAY_SECS
}

/// Serde default function for [`UserPreferences::stall_retry_max_delay_secs`].
pub(crate) fn default_stall_retry_max_delay_secs() -> u64 {
    DEFAULT_STALL_RETRY_MAX_DELAY_SECS
}
/// Default execution timeout (seconds) for all tool calls.
///
/// The model can override per-call via the reserved `max_duration_secs` argument
/// (supported by `bash`); a value of `0` disables the timeout for that call.
pub(crate) const DEFAULT_TOOL_DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Serde default function for [`UserPreferences::tool_default_timeout_secs`].
pub(crate) fn default_tool_default_timeout_secs() -> u64 {
    DEFAULT_TOOL_DEFAULT_TIMEOUT_SECS
}

/// Errors that can occur during user preferences I/O.
#[derive(Debug, Error)]
pub enum UserPreferencesError {
    /// Filesystem I/O failure.
    #[error("user preferences I/O error")]
    Io,
    /// TOML parsing or structural error.
    #[error("user preferences parse error")]
    Parse,
}

/// User preferences persisted in `jinn.toml`.
///
/// This file stores user behavior preferences that should survive
/// app restarts - e.g., the last model and strategy selected from pickers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserPreferences {
    /// Maximum number of lines to display for tool entries in the chat log.
    /// `None` means use the built-in default (5 lines).
    #[serde(default)]
    pub tool_entry_max_lines: Option<u16>,
    /// Minimum number of contiguous excluded entries required to collapse into
    /// a summary line. `None` means use the built-in default (3).
    #[serde(default)]
    pub min_collapse_count: Option<usize>,

    /// Named session lifecycle recipes - paired setup/teardown commands.
    /// The implicit "blank" lifecycle (no commands) is always available and
    /// does not need to be listed here.
    #[serde(default)]
    #[serde(rename = "session_lifecycle")]
    pub session_lifecycles: Vec<SessionLifecycle>,

    /// Curated project directories shown in the project picker.
    /// These are purely user-curated (no auto-tracking); the user adds/removes
    /// entries explicitly. See [`ProjectConfig`].
    #[serde(default)]
    pub projects: Vec<ProjectConfig>,

    /// Maximum number of lines for tool output before truncation.
    /// `None` means use the built-in default (2000 lines).
    #[serde(default)]
    pub max_tool_output_lines: Option<usize>,
    /// Maximum size in bytes for tool output before truncation.
    /// `None` means use the built-in default (50KB).
    #[serde(default)]
    pub max_tool_output_bytes: Option<usize>,
    /// Compaction configuration.
    #[serde(default)]
    pub compaction: CompactionConfig,
    /// Retry configuration for LLM provider requests.
    #[serde(default)]
    pub request_retry: RequestRetryConfig,
    /// Web fetch tool configuration.
    #[serde(default)]
    pub web_fetch: WebFetchConfig,
    /// Web search tool configuration.
    #[serde(default)]
    pub web_search: WebSearchConfig,
    /// OpenRouter web search server tool configuration.
    #[serde(default)]
    pub openrouter_web_search: OpenrouterWebSearchConfig,
    /// CWD selector configuration.
    #[serde(default)]
    pub cwd_selector: CwdSelectorConfig,
    /// Minimap configuration.
    #[serde(default)]
    pub minimap: MinimapConfig,
    /// Auto-prune configuration.
    #[serde(default)]
    pub auto_prune: AutoPruneConfig,
    /// Discord bot configuration. Off by default.
    #[serde(default)]
    pub discord: crate::feat::discord::DiscordConfig,
    /// Default execution timeout (seconds) for all tool calls.
    ///
    /// The model can override per-call via the reserved `max_duration_secs` argument
    /// (supported by `bash`); a value of `0` disables the timeout for that call.
    #[serde(default = "default_tool_default_timeout_secs")]
    pub tool_default_timeout_secs: u64,

    /// Maximum seconds without a chat-history change (new entry, appended
    /// token, appended thinking token) before a session in `Sending` or
    /// `Streaming` is considered hung and retried.
    #[serde(default = "default_history_stall_timeout_secs")]
    pub history_stall_timeout_secs: u64,

    /// Maximum stall retries before the watchdog gives up and cancels the turn.
    /// Independent of `[request_retry]` max_retries.
    #[serde(default = "default_stall_retry_max_retries")]
    pub stall_retry_max_retries: u32,

    /// Base delay (seconds) for stall-retry exponential backoff. The watchdog
    /// waits at least this long (scaled by `2^attempt` with full jitter, capped
    /// by `stall_retry_max_delay_secs`) between consecutive retries of the same
    /// stalled session. Mirrors the `[request_retry]` backoff shape.
    #[serde(default = "default_stall_retry_base_delay_secs")]
    pub stall_retry_base_delay_secs: u64,

    /// Maximum cap (seconds) for stall-retry exponential backoff.
    #[serde(default = "default_stall_retry_max_delay_secs")]
    pub stall_retry_max_delay_secs: u64,
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            tool_entry_max_lines: None,
            min_collapse_count: None,
            session_lifecycles: vec![SessionLifecycle {
                name: "git worktree".to_owned(),
                description: Some("Open a git worktree + branch".to_owned()),
                setup: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "git worktree add -b <branch> ../<branch> && echo ../<branch>"
                            .to_owned(),
                    ),
                ),
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "git merge <branch> && git worktree remove ../<branch> && git branch -d <branch>"
                            .to_owned(),
                    ),
                ),
            }],
            projects: vec![],
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            web_search: WebSearchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            discord: crate::feat::discord::DiscordConfig::default(),
            tool_default_timeout_secs: default_tool_default_timeout_secs(),
            history_stall_timeout_secs: default_history_stall_timeout_secs(),
            stall_retry_max_retries: default_stall_retry_max_retries(),
            stall_retry_base_delay_secs: default_stall_retry_base_delay_secs(),
            stall_retry_max_delay_secs: default_stall_retry_max_delay_secs(),
        }
    }
}

/// Returns the path to the user preferences file.
///
/// Uses `dirs::config_dir()` → `~/.config/jinn/jinn.toml`.
#[must_use]
pub fn preferences_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_NAME)
        .join(PREFS_FILE_NAME)
}

/// Loads user preferences from the default path.
///
/// Returns default preferences if the file does not exist.
///
/// # Errors
///
/// Returns [`UserPreferencesError::Parse`] if the TOML is malformed.
/// Returns [`UserPreferencesError::Io`] if the file cannot be read.
pub fn load_preferences() -> Result<UserPreferences, Report<UserPreferencesError>> {
    load_preferences_from(preferences_path())
}

/// Loads preferences from a specific path.
///
/// If the path does not exist, the canonical default template
/// (`DEFAULT_CONFIG`) is written there first so the user gets a
/// comment-rich starter file, then parsed.
pub(crate) fn load_preferences_from<P>(
    path: P,
) -> Result<UserPreferences, Report<UserPreferencesError>>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();

    if !path.exists() {
        create_default_preferences_to(path)?;
    }

    let content = std::fs::read_to_string(path)
        .change_context(UserPreferencesError::Io)
        .attach("failed to read user preferences")?;

    toml::from_str(&content)
        .change_context(UserPreferencesError::Parse)
        .attach("failed to parse user preferences")
}

/// Writes the canonical default preferences template to `path`.
///
/// Creates parent directories as needed.
///
/// # Errors
///
/// Returns [`UserPreferencesError::Io`] if directory creation or file writing fails.
pub(crate) fn create_default_preferences_to<P>(path: P) -> Result<(), Report<UserPreferencesError>>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .change_context(UserPreferencesError::Io)
            .attach("failed to create preferences directory")?;
    }

    std::fs::write(path, DEFAULT_CONFIG)
        .change_context(UserPreferencesError::Io)
        .attach("failed to write default user preferences")
}

/// Error returned by [`init_default_config_to`].
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct InitDefaultConfigError;

/// Outcome of [`init_default_config_to`].
#[derive(Debug)]
pub enum InitOutcome {
    /// Template was written to a previously-missing path.
    Created,
    /// Existing file was overwritten (caller passed `force: true`).
    Overwritten,
}

/// Writes [`DEFAULT_CONFIG`] to `path`.
///
/// - If `path` does not exist: writes the template, returns [`InitOutcome::Created`].
/// - If `path` exists and `force` is false: returns `Err(InitDefaultConfigError)`.
/// - If `path` exists and `force` is true: overwrites, returns [`InitOutcome::Overwritten`].
///
/// Creates parent directories as needed.
///
/// # Errors
///
/// Returns [`Report<InitDefaultConfigError>`] if the file already exists and
/// `force` is false, or if directory creation / file writing fails.
pub fn init_default_config_to<P>(
    path: P,
    force: bool,
) -> Result<InitOutcome, Report<InitDefaultConfigError>>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();
    let existed = path.exists();

    if existed && !force {
        return Err(Report::new(InitDefaultConfigError))
            .attach("jinn.toml already exists; pass --force to overwrite")
            .attach(format!("path: {}", path.display()));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .change_context(InitDefaultConfigError)
            .attach("failed to create preferences directory")?;
    }

    std::fs::write(path, DEFAULT_CONFIG)
        .change_context(InitDefaultConfigError)
        .attach("failed to write default user preferences")?;

    if existed {
        Ok(InitOutcome::Overwritten)
    } else {
        Ok(InitOutcome::Created)
    }
}

/// Saves preferences to the default path.
///
/// Creates parent directories as needed.
///
/// # Errors
///
/// Returns [`UserPreferencesError::Parse`] if serialization fails.
/// Returns [`UserPreferencesError::Io`] if writing fails.
pub fn save_preferences(prefs: &UserPreferences) -> Result<(), Report<UserPreferencesError>> {
    save_preferences_to(prefs, preferences_path())
}

/// Saves preferences to a specific path.
pub(crate) fn save_preferences_to<P>(
    prefs: &UserPreferences,
    path: P,
) -> Result<(), Report<UserPreferencesError>>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .change_context(UserPreferencesError::Io)
            .attach("failed to create preferences directory")?;
    }

    // If the file exists, patch it in place to preserve user comments / ordering.
    // Otherwise, emit a clean zero-comment serialization (no template to keep in sync).
    let content = if path.exists() {
        let existing = std::fs::read_to_string(path)
            .change_context(UserPreferencesError::Io)
            .attach("failed to read existing jinn.toml")?;

        let mut doc: toml_edit::DocumentMut =
            existing.parse().map_err(|err: toml_edit::TomlError| {
                Report::new(UserPreferencesError::Parse)
                    .attach("failed to parse existing jinn.toml")
                    .attach(err.to_string())
            })?;

        let new_value = toml::Value::try_from(prefs)
            .change_context(UserPreferencesError::Parse)
            .attach("failed to serialize UserPreferences")?;

        let toml::Value::Table(new_table) = &new_value else {
            return Err(Report::new(UserPreferencesError::Parse)
                .attach("UserPreferences serialized to non-table TOML value"));
        };
        let mut patcher = DocumentPatcher::new();
        patcher.register_array_key(["session_lifecycle"], "name");
        patcher.register_array_key(["auto_prune", "regex", "rules"], "pattern");
        patcher.register_array_key(["project"], "path");

        patcher
            .apply(new_table, doc.as_table_mut())
            .change_context(UserPreferencesError::Parse)
            .attach("failed to patch jinn.toml document")?;

        doc.to_string()
    } else {
        toml::to_string_pretty(prefs)
            .change_context(UserPreferencesError::Parse)
            .attach("failed to serialize user preferences")?
    };

    std::fs::write(path, content)
        .change_context(UserPreferencesError::Io)
        .attach("failed to write user preferences")
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::print_stderr,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use tempfile::TempDir;

    use super::*;

    #[rstest::rstest]
    fn default_tool_timeout_is_300_seconds() {
        // Given the global default timeout constant.
        // Then it is 300 seconds.
        assert_eq!(DEFAULT_TOOL_DEFAULT_TIMEOUT_SECS, 300);
        // And the serde default function agrees.
        assert_eq!(default_tool_default_timeout_secs(), 300);
    }

    #[rstest::rstest]
    fn default_preferences_has_defaults_for_optional_fields() {
        // Given default preferences.
        let prefs = UserPreferences::default();

        // Then optional fields default to None.
        assert!(prefs.tool_entry_max_lines.is_none());
        assert!(prefs.min_collapse_count.is_none());
    }

    #[rstest::rstest]
    fn load_returns_defaults_and_creates_file_when_missing() {
        // Given a path to a nonexistent file.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then defaults are returned.

        assert!(prefs.tool_entry_max_lines.is_none());
        // And the file is created.
        assert!(path.exists());
    }

    #[rstest::rstest]
    fn load_creates_file_with_template_bytes_when_missing() {
        // Given a path to a nonexistent file.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);

        // When loading.
        load_preferences_from(&path).expect("load");

        // Then the file's bytes are exactly the embedded template.
        let on_disk = std::fs::read_to_string(&path).expect("read");
        assert_eq!(on_disk, DEFAULT_CONFIG);
    }

    #[rstest::rstest]
    fn load_does_not_touch_existing_file() {
        // Given an existing file with custom content.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let marker = "# user-managed\ntool_entry_max_lines = 10\n";
        std::fs::write(&path, marker).expect("write");
        let mtime_before = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .expect("metadata");

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then the file on disk is unchanged.
        let on_disk = std::fs::read_to_string(&path).expect("read");
        assert_eq!(on_disk, marker);
        // And the parsed prefs reflect the file, not the defaults.
        assert_eq!(prefs.tool_entry_max_lines, Some(10));
        // And the mtime is preserved.
        let mtime_after = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .expect("metadata");
        assert_eq!(mtime_before, mtime_after);
    }

    #[rstest::rstest]
    fn default_config_template_round_trips_to_user_preferences_default() {
        // Given the shipped default_jinn.toml template.
        // When checking it against UserPreferences::default().
        let result = crate::common::default_config_check::check_default_round_trips_to_default::<
            UserPreferences,
        >(DEFAULT_CONFIG);

        // Then the template deserializes to the inherent default with no drift.
        assert!(
            result.is_ok(),
            "default_jinn.toml has drifted from UserPreferences::default(): {result:?}",
        );
    }

    #[rstest::rstest]
    fn init_writes_template_when_missing() {
        // Given a path to a nonexistent file.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);

        // When initializing the default config (no force).
        let outcome = init_default_config_to(&path, false).expect("init");

        // Then the file is created with the template bytes.
        assert!(matches!(outcome, InitOutcome::Created));
        let on_disk = std::fs::read_to_string(&path).expect("read");
        assert_eq!(on_disk, DEFAULT_CONFIG);
    }

    #[rstest::rstest]
    fn init_returns_already_exists_when_present_and_no_force() {
        // Given an existing file with custom content.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let marker = "# user-managed\ntool_entry_max_lines = 10\n";
        std::fs::write(&path, marker).expect("write");

        // When initializing without --force.
        let result = init_default_config_to(&path, false);

        // Then the call fails with InitDefaultConfigError.
        assert!(result.is_err());
        // And the file is unchanged.
        let on_disk = std::fs::read_to_string(&path).expect("read");
        assert_eq!(on_disk, marker);
    }

    #[rstest::rstest]
    fn init_overwrites_when_force() {
        // Given an existing file with custom content.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, "# stale\n").expect("write");

        // When initializing with --force.
        let outcome = init_default_config_to(&path, true).expect("init");

        // Then the file is overwritten with the template bytes.
        assert!(matches!(outcome, InitOutcome::Overwritten));
        let on_disk = std::fs::read_to_string(&path).expect("read");
        assert_eq!(on_disk, DEFAULT_CONFIG);
    }

    #[rstest::rstest]
    fn save_then_load_round_trips() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            tool_entry_max_lines: Some(10),
            min_collapse_count: None,
            session_lifecycles: vec![],
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            web_search: WebSearchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            projects: vec![],
            discord: crate::feat::discord::DiscordConfig::default(),
            tool_default_timeout_secs: default_tool_default_timeout_secs(),
            history_stall_timeout_secs: default_history_stall_timeout_secs(),
            stall_retry_max_retries: default_stall_retry_max_retries(),
            stall_retry_base_delay_secs: default_stall_retry_base_delay_secs(),
            stall_retry_max_delay_secs: default_stall_retry_max_delay_secs(),
        };

        // When saving and reloading.
        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");

        // Then the round-tripped data matches.
        assert_eq!(reloaded.tool_entry_max_lines, Some(10));
    }

    #[rstest::rstest]
    fn load_parses_toml_content() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, "tool_entry_max_lines = 10").expect("write");
        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        assert_eq!(prefs.tool_entry_max_lines, Some(10));
    }

    #[rstest::rstest]
    fn load_handles_empty_file() {
        // Given an empty TOML file.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, "").expect("write");

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then defaults are returned (all fields None).

        assert!(prefs.tool_entry_max_lines.is_none());
    }

    #[rstest::rstest]
    fn save_creates_parent_directories() {
        // Given a nested path that doesn't exist.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("nested").join("dir").join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            tool_entry_max_lines: None,
            min_collapse_count: None,
            session_lifecycles: vec![],
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            web_search: WebSearchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            projects: vec![],
            discord: crate::feat::discord::DiscordConfig::default(),
            tool_default_timeout_secs: default_tool_default_timeout_secs(),
            history_stall_timeout_secs: default_history_stall_timeout_secs(),
            stall_retry_max_retries: default_stall_retry_max_retries(),
            stall_retry_base_delay_secs: default_stall_retry_base_delay_secs(),
            stall_retry_max_delay_secs: default_stall_retry_max_delay_secs(),
        };

        // When saving.
        save_preferences_to(&prefs, &path).expect("save");

        // Then the file exists.
        assert!(path.exists());
    }

    #[rstest::rstest]
    fn save_then_load_round_trips_tool_entry_max_lines() {
        // Given preferences with a tool_entry_max_lines override.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            tool_entry_max_lines: Some(10),
            min_collapse_count: None,
            session_lifecycles: vec![],
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            web_search: WebSearchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            projects: vec![],
            discord: crate::feat::discord::DiscordConfig::default(),
            tool_default_timeout_secs: default_tool_default_timeout_secs(),
            history_stall_timeout_secs: default_history_stall_timeout_secs(),
            stall_retry_max_retries: default_stall_retry_max_retries(),
            stall_retry_base_delay_secs: default_stall_retry_base_delay_secs(),
            stall_retry_max_delay_secs: default_stall_retry_max_delay_secs(),
        };

        // When saving and reloading.
        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");

        // Then the round-tripped value matches.
        assert_eq!(reloaded.tool_entry_max_lines, Some(10));
    }

    #[rstest::rstest]
    fn default_preferences_has_git_worktree_lifecycle() {
        // Given default preferences.
        let prefs = UserPreferences::default();

        // Then session_lifecycles contains the git worktree lifecycle.
        assert_eq!(prefs.session_lifecycles.len(), 1);
        assert_eq!(prefs.session_lifecycles[0].name, "git worktree");
        assert!(prefs.session_lifecycles[0].setup.is_some());
    }

    #[rstest::rstest]
    fn preferences_path_ends_with_jinn_toml() {
        // Given the standard preferences path.
        let path = preferences_path();

        // Then it ends with jinn/jinn.toml.
        assert!(path.to_string_lossy().ends_with("jinn/jinn.toml"));
    }

    #[rstest::rstest]
    fn save_then_load_round_trips_min_collapse_count() {
        // Given preferences with a min_collapse_count override.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            min_collapse_count: Some(5),
            ..UserPreferences::default()
        };

        // When saving and reloading.
        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");

        // Then the round-tripped value matches.
        assert_eq!(reloaded.min_collapse_count, Some(5));
    }

    #[rstest::rstest]
    fn save_preferences_preserves_user_comments_on_scalar_change() {
        // Given a comment-rich jinn.toml.
        let original = "# my prefs\ntool_entry_max_lines = 10\n";
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, original).expect("write");

        // When loading, mutating tool_entry_max_lines, and saving.
        let mut prefs = load_preferences_from(&path).expect("load");
        prefs.tool_entry_max_lines = Some(20);
        save_preferences_to(&prefs, &path).expect("save");

        // Then the comment is preserved and the field is updated.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(written.contains("# my prefs"), "comment wiped: {written}");
        assert!(written.contains("tool_entry_max_lines = 20"));
        assert!(!written.contains("tool_entry_max_lines = 10"));
    }

    #[rstest::rstest]
    fn first_save_of_jinn_toml_emits_no_comments() {
        // Given: no jinn.toml exists yet.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        assert!(!path.exists());

        // When: saving for the first time.
        let prefs = UserPreferences::default();
        save_preferences_to(&prefs, &path).expect("save");

        // Then: the written file contains no comment characters at all.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(
            !written.contains('#'),
            "first save of jinn.toml must be comment-free, got: {written}"
        );
    }

    #[rstest::rstest]
    fn save_preferences_preserves_user_comments_in_auto_prune_section() {
        // Given a jinn.toml with comments in the auto_prune.regex section.
        let original = "# auto-prune rules\n[auto_prune.regex]\nenabled = true\n\n# matches foo\n[[auto_prune.regex.rules]]\npattern = \"foo\"\nkeep_last = 3\n";
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, original).expect("write");

        // When loading, mutating keep_last, and saving.
        let mut prefs = load_preferences_from(&path).expect("load");
        if let Some(rule) = prefs.auto_prune.regex.rules.iter_mut().next() {
            rule.keep_last = 99;
        }
        save_preferences_to(&prefs, &path).expect("save");

        // Then the comments are preserved and the value is updated.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(written.contains("# auto-prune rules"));
        assert!(written.contains("# matches foo"));
        assert!(written.contains("keep_last = 99"));
    }
    #[rstest::rstest]
    fn save_preferences_comprehensive_comment_round_trip_preserves_all_styles() {
        // Given a jinn.toml fixture using every comment style we promise to
        // preserve: top-of-file banner, section header, mid-table inline,
        // array-of-tables block headers.
        let original = r#"# my jinn preferences - hand-edited
                                                            
        # main prefs
        last_model = "openrouter/anthropic/claude-sonnet-4-20250514"
                                                            
        # compaction
        [compaction]
        enabled = true        # always compact
        threshold = 100       # tokens
                                                            
        # session lifecycles
        [[session_lifecycle]]
        name = "fossil-branch"
        description = "Open a fossil branch in a new workdir"
                                                            
        [[session_lifecycle]]
        name = "cleanup"
        description = "Tidy up after session"
                                                            
        # auto-prune
        [auto_prune.regex]
        enabled = true
                                                            
        # matches todo-related files
        [[auto_prune.regex.rules]]
        pattern = "TODO\\.md"
        keep_last = 2
                                                            
        # matches build artifacts
        [[auto_prune.regex.rules]]
        pattern = "target/"
        keep_last = 1
        "#;
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, original).expect("write");

        // When loading and immediately re-saving without changes.
        let prefs = load_preferences_from(&path).expect("load");
        save_preferences_to(&prefs, &path).expect("save");

        // Then every comment style is preserved byte-for-byte.
        let written = std::fs::read_to_string(&path).expect("read");
        for expected in [
            "# my jinn preferences - hand-edited",
            "# main prefs",
            "# compaction",
            "# always compact", // inline trailing
            "# tokens",         // inline trailing
            "# session lifecycles",
            "# auto-prune",
            "# matches todo-related files",
            "# matches build artifacts",
        ] {
            assert!(
                written.contains(expected),
                "comment lost: {expected:?}\nGot:\n{written}"
            );
        }
    }

    #[rstest::rstest]
    fn save_preferences_mixed_mutations_preserve_unrelated_comments() {
        // Given a jinn.toml with comments sprinkled across several sections.
        let original = "\
# main preferences\ntool_entry_max_lines = 10\n\n# collapse threshold\nmin_collapse_count = 5\n\n# keep context compact\n[compaction]\n# always compact\nenabled = true\n# 50k tokens\ntokens = 50000\n\n# my lifecycles\n[[session_lifecycle]]\nname = \"alpha\"\n\n# deprecated lifecycle\n[[session_lifecycle]]\nname = \"beta\"\n";
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, original).expect("write");

        // When applying a mixed mutation set:
        //   - change tool_entry_max_lines (scalar update)
        //   - delete beta session_lifecycle (array entry removal)
        //   - leave min_collapse_count and compaction untouched
        let mut prefs = load_preferences_from(&path).expect("load");
        prefs.tool_entry_max_lines = Some(20);
        prefs.session_lifecycles.retain(|l| l.name == "alpha");
        save_preferences_to(&prefs, &path).expect("save");

        // Then all unrelated comments survive and the targeted changes applied.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(written.contains("# main preferences"), "top comment kept");
        assert!(
            written.contains("# collapse threshold"),
            "collapse comment kept"
        );
        assert!(
            written.contains("min_collapse_count = 5"),
            "untouched field kept"
        );
        assert!(
            written.contains("# keep context compact"),
            "compaction comment kept"
        );
        assert!(written.contains("# always compact"), "nested comment kept");
        assert!(
            written.contains("# 50k tokens"),
            "second nested comment kept"
        );
        assert!(
            written.contains("# my lifecycles"),
            "lifecycles comment kept"
        );
        assert!(
            !written.contains("# deprecated lifecycle"),
            "beta comment removed with beta"
        );
        assert!(!written.contains("\"beta\""), "beta removed");
        assert!(
            written.contains("tool_entry_max_lines = 20"),
            "tool_entry_max_lines updated"
        );
        assert!(
            !written.contains("tool_entry_max_lines = 10"),
            "old tool_entry_max_lines gone"
        );
        // The alpha lifecycle is preserved.
        assert!(written.contains("\"alpha\""), "alpha kept");
    }
    #[rstest::rstest]
    fn save_preferences_preserves_inner_block_comment_when_field_is_mutated() {
        // Given a jinn.toml with a comment between two session_lifecycle fields.
        // (The comment attaches to the next field's key decor, not its value.)
        let original = "[[session_lifecycle]]\nname = \"cwd test\"\n# am i preserved?\ndescription = \"Open a fossil branch in a new worktree\"\n";
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, original).expect("write");

        // When loading and mutating the field after the comment.
        let mut prefs = load_preferences_from(&path).expect("load");
        prefs.session_lifecycles[0].description = Some("UPDATED DESCRIPTION".to_owned());
        save_preferences_to(&prefs, &path).expect("save");

        // Then the inner comment survives AND the field is updated.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(
            written.contains("# am i preserved?"),
            "inner comment lost on mutation:\n{written}"
        );
        assert!(
            written.contains("UPDATED DESCRIPTION"),
            "description updated:\n{written}"
        );
    }

    #[rstest::rstest]
    fn load_preferences_actually_reads_file_content() {
        // If load_preferences were a no-op returning defaults, this would fail
        // because we verify that file content is actually read and parsed.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "tool_entry_max_lines = 42\n             min_collapse_count = 7\n",
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");

        // Then the loaded prefs are NOT defaults - they reflect the file.
        assert_eq!(prefs.tool_entry_max_lines, Some(42));
        assert_eq!(prefs.min_collapse_count, Some(7));
    }

    #[rstest::rstest]
    fn save_preferences_actually_writes_to_disk() {
        // If save_preferences were a no-op, the file would not exist on disk.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            tool_entry_max_lines: Some(99),
            min_collapse_count: None,
            session_lifecycles: vec![],
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            web_search: WebSearchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            projects: vec![],
            discord: crate::feat::discord::DiscordConfig::default(),
            tool_default_timeout_secs: default_tool_default_timeout_secs(),
            history_stall_timeout_secs: default_history_stall_timeout_secs(),
            stall_retry_max_retries: default_stall_retry_max_retries(),
            stall_retry_base_delay_secs: default_stall_retry_base_delay_secs(),
            stall_retry_max_delay_secs: default_stall_retry_max_delay_secs(),
        };

        save_preferences_to(&prefs, &path).expect("save");

        // Then the file exists on disk with the expected content.
        assert!(path.exists(), "save_preferences should create the file");
        let content = std::fs::read_to_string(&path).expect("read back");
        assert!(content.contains("tool_entry_max_lines = 99"));
        assert!(content.contains("99"));
    }

    #[rstest::rstest]
    fn save_preferences_preserves_user_comments() {
        // Given a comment-rich jinn.toml.
        let original = "# my favorite\ntool_entry_max_lines = 10\nmin_collapse_count = 42\n";
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, original).expect("write");

        // When loading, mutating tool_entry_max_lines, and saving.
        let mut prefs = load_preferences_from(&path).expect("load");
        prefs.tool_entry_max_lines = Some(7);
        save_preferences_to(&prefs, &path).expect("save");

        // Then the comment is preserved verbatim.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(
            written.contains("# my favorite"),
            "comment was wiped: {written}"
        );
        assert!(written.contains("tool_entry_max_lines = 7"));
        assert!(written.contains("min_collapse_count = 42"));
    }

    #[rstest::rstest]
    fn default_preferences_has_headless_chrome_web_fetch() {
        let prefs = UserPreferences::default();
        assert_eq!(prefs.web_fetch.backend, WebFetchBackend::HeadlessChrome);
    }

    #[rstest::rstest]
    fn default_preferences_has_default_openrouter_web_search() {
        let prefs = UserPreferences::default();
        assert_eq!(prefs.openrouter_web_search.engine.as_deref(), Some("exa"));
        assert!(prefs.openrouter_web_search.max_results.is_none());
        assert!(prefs.openrouter_web_search.max_total_results.is_none());
        assert!(prefs.openrouter_web_search.search_context_size.is_none());
        assert!(prefs.openrouter_web_search.allowed_domains.is_none());
        assert!(prefs.openrouter_web_search.excluded_domains.is_none());
    }
    #[rstest::rstest]
    fn default_preferences_has_default_web_search_config() {
        // Given default preferences.
        let prefs = UserPreferences::default();

        // Then web_search config uses defaults.
        assert_eq!(prefs.web_search.max_results, 10);
        assert_eq!(prefs.web_search.region, "wt-wt");
        assert!(prefs.web_search.safe_search);
    }
    #[rstest::rstest]
    fn default_preferences_has_default_minimap_config() {
        // Given default preferences.
        let prefs = UserPreferences::default();

        // Then minimap config uses defaults.
        assert_eq!(prefs.minimap.max_tokens, 2000);
    }

    #[rstest::rstest]
    fn default_preferences_has_default_auto_prune_config() {
        let prefs = UserPreferences::default();
        assert!(prefs.auto_prune.read_edit.enabled);
        assert!(prefs.auto_prune.edit_read.enabled);
        assert!(prefs.auto_prune.todo.enabled);
        assert!(prefs.auto_prune.consecutive_reads.enabled);
        assert!(prefs.auto_prune.tool_age_window.enabled);
        assert!(prefs.auto_prune.trivial_assistant.enabled);
        assert_eq!(prefs.auto_prune.trivial_assistant.min_age, 100);
        assert_eq!(prefs.auto_prune.trivial_assistant.max_tokens, 80);
    }

    #[rstest::rstest]
    fn default_history_stall_timeout_is_sixty_seconds() {
        // Given default preferences.
        let prefs = UserPreferences::default();

        // Then the stall timeout is 60 seconds (tightened from 300 to catch
        // transient provider death faster without false-positiving on slow
        // but responsive providers).
        assert_eq!(prefs.history_stall_timeout_secs, 60);
    }
}
