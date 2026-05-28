//! User preferences data type and file I/O.
//!
//! Defines [`UserPreferences`] as the schema for `nullslop.toml`,
//! along with loading and saving logic. The file lives at
//! `~/.config/nullslop/nullslop.toml` and is auto-created on first save.

use std::path::{Path, PathBuf};

use crate::common::app_info::{APP_NAME, PREFS_FILE_NAME};
use error_stack::{Report, ResultExt as _};
use serde::{Deserialize, Serialize};
use wherror::Error;

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

/// A named session lifecycle recipe — paired setup and teardown commands.
///
/// Defined in `nullslop.toml` under `[[session_lifecycle]]`. The setup command
/// runs when creating a new session; the teardown command runs when closing it.
/// Commands may contain positional parameters (`$1`, `$2`) that are collected
/// from the user before execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionLifecycle {
    /// Human-readable name shown in the lifecycle picker.
    pub name: String,
    /// Optional description shown below the name in the picker.
    #[serde(default)]
    pub description: Option<String>,
    /// Command to run when creating a session. Last line of stdout becomes the CWD.
    /// May contain `$1`, `$2` positional args. `None` means no setup (blank lifecycle).
    ///
    /// Supports both shell commands and builtin handlers.
    /// See [`LifecycleCommand`] for details.
    #[serde(rename = "setup_command", default)]
    pub setup: Option<crate::feat::session_lifecycle::builtin::LifecycleCommand>,
    /// Command to run when closing a session. Receives the same args as setup.
    /// `None` means no teardown needed.
    ///
    /// Supports both shell commands and builtin handlers.
    /// See [`LifecycleCommand`] for details.
    #[serde(rename = "teardown_command", default)]
    pub teardown: Option<crate::feat::session_lifecycle::builtin::LifecycleCommand>,
}

/// Default token threshold for auto-compaction.
const DEFAULT_COMPACTION_THRESHOLD: f64 = 0.7;

/// Default number of recent tokens to reserve from compaction.
const DEFAULT_RESERVE_TOKENS: usize = 20_000;

/// Default fallback context window when the provider doesn't report one.
const DEFAULT_FALLBACK_CONTEXT_WINDOW: usize = 150_000;

/// Compaction configuration.
///
/// Serialized as `[compaction]` in `nullslop.toml`.
/// Controls when and how context compaction summarizes conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Provider/model for compaction summarization (e.g., "anthropic/claude-sonnet-4-20250514").
    /// Falls back to the session model if not set or if provider construction fails.
    #[serde(default)]
    pub model: Option<String>,
    /// Fraction of context window at which auto-compaction triggers (0.0–1.0).
    /// Default: 0.7 (70% of budget).
    #[serde(default = "default_compaction_threshold")]
    pub threshold: f64,
    /// Number of recent tokens to reserve from compaction.
    /// Default: 20,000.
    #[serde(default = "default_reserve_tokens")]
    pub reserve_tokens: usize,
    /// Fallback context window size when the provider doesn't report `context_length`.
    /// Used for auto-compaction threshold calculation with local models (Ollama, LM Studio).
    /// Default: 150,000.
    #[serde(default = "default_fallback_context_window")]
    pub fallback_context_window: usize,
}

fn default_compaction_threshold() -> f64 {
    DEFAULT_COMPACTION_THRESHOLD
}

fn default_reserve_tokens() -> usize {
    DEFAULT_RESERVE_TOKENS
}

fn default_fallback_context_window() -> usize {
    DEFAULT_FALLBACK_CONTEXT_WINDOW
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            model: None,
            threshold: DEFAULT_COMPACTION_THRESHOLD,
            reserve_tokens: DEFAULT_RESERVE_TOKENS,
            fallback_context_window: DEFAULT_FALLBACK_CONTEXT_WINDOW,
        }
    }
}

/// Default sliding window size.
const DEFAULT_SLIDING_WINDOW_SIZE: usize = 5;

/// Sliding window configuration.
///
/// Serialized as `[context_sliding_window]` in `nullslop.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSlidingWindowConfig {
    /// The default window size for new sessions using the sliding-window strategy.
    #[serde(default = "default_sliding_window_size")]
    pub size: usize,
}

fn default_sliding_window_size() -> usize {
    DEFAULT_SLIDING_WINDOW_SIZE
}

impl Default for ContextSlidingWindowConfig {
    fn default() -> Self {
        Self {
            size: DEFAULT_SLIDING_WINDOW_SIZE,
        }
    }
}

/// Web fetch backend selection.
///
/// Determines which fetching strategy is used for the `web-fetch` tool.
/// Selected once at startup from `nullslop.toml` and never changes at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WebFetchBackend {
    /// Plain HTTP requests via `reqwest`. No JavaScript rendering.
    #[default]
    Http,
    /// Headless Chrome browser via `headless_chrome` crate. Renders JavaScript.
    HeadlessChrome,
}

/// Web fetch tool configuration.
///
/// Serialized as `[web_fetch]` in `nullslop.toml`.
/// Controls which backend the `web-fetch` tool uses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebFetchConfig {
    /// The backend to use for web fetching. Default: `"http"`.
    #[serde(default)]
    pub backend: WebFetchBackend,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            backend: WebFetchBackend::Http,
        }
    }
}

/// OpenRouter web search server tool configuration.
///
/// Serialized as `[openrouter_web_search]` in `nullslop.toml`.
/// Controls parameters sent to the `openrouter:web_search` server tool.
/// All fields are optional — when `None`, the parameter is omitted from
/// the request and OpenRouter uses its default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenrouterWebSearchConfig {
    /// Search engine: "auto", "native", "exa", "firecrawl", or "parallel".
    /// Default: "exa".
    #[serde(default)]
    pub engine: Option<String>,

    /// Maximum results per search call (1–25). `None` = OpenRouter default (5).
    #[serde(default)]
    pub max_results: Option<u32>,

    /// Maximum total results across all searches in one request.
    #[serde(default)]
    pub max_total_results: Option<u32>,

    /// How much context to retrieve: "low", "medium", or "high".
    /// `None` = OpenRouter picks adaptively.
    #[serde(default)]
    pub search_context_size: Option<String>,

    /// Only return results from these domains.
    #[serde(default)]
    pub allowed_domains: Option<Vec<String>>,

    /// Exclude results from these domains.
    #[serde(default)]
    pub excluded_domains: Option<Vec<String>>,
}

impl Default for OpenrouterWebSearchConfig {
    fn default() -> Self {
        Self {
            engine: Some("exa".to_owned()),
            max_results: None,
            max_total_results: None,
            search_context_size: None,
            allowed_domains: None,
            excluded_domains: None,
        }
    }
}

/// Default retry configuration values.
const DEFAULT_RETRY_MAX_RETRIES: u32 = 5;
const DEFAULT_RETRY_BASE_DELAY_SECS: u64 = 2;
const DEFAULT_RETRY_MAX_DELAY_SECS: u64 = 60;

/// Retry configuration for LLM provider requests.
///
/// Serialized as `[request_retry]` in `nullslop.toml`.
/// Controls exponential backoff behavior for transient errors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestRetryConfig {
    /// Maximum number of retry attempts. Default: 5.
    #[serde(default = "default_retry_max_retries")]
    pub max_retries: u32,
    /// Base delay in seconds for exponential backoff. Default: 2.
    #[serde(default = "default_retry_base_delay_secs")]
    pub base_delay_secs: u64,
    /// Maximum delay cap in seconds. Default: 60.
    /// Overridden by provider-supplied Retry-After / error body hints.
    #[serde(default = "default_retry_max_delay_secs")]
    pub max_delay_secs: u64,
}

fn default_retry_max_retries() -> u32 {
    DEFAULT_RETRY_MAX_RETRIES
}
fn default_retry_base_delay_secs() -> u64 {
    DEFAULT_RETRY_BASE_DELAY_SECS
}
fn default_retry_max_delay_secs() -> u64 {
    DEFAULT_RETRY_MAX_DELAY_SECS
}

impl Default for RequestRetryConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_RETRY_MAX_RETRIES,
            base_delay_secs: DEFAULT_RETRY_BASE_DELAY_SECS,
            max_delay_secs: DEFAULT_RETRY_MAX_DELAY_SECS,
        }
    }
}

impl RequestRetryConfig {
    /// Convert to the provider-crate [`nullslop_provider::RetryConfig`].
    #[must_use]
    pub fn to_retry_config(&self) -> nullslop_provider::RetryConfig {
        nullslop_provider::RetryConfig {
            max_retries: self.max_retries,
            base_delay: std::time::Duration::from_secs(self.base_delay_secs),
            max_delay: std::time::Duration::from_secs(self.max_delay_secs),
        }
    }
}

/// User preferences persisted in `nullslop.toml`.
///
/// This file stores user behavior preferences that should survive
/// app restarts — e.g., the last model and strategy selected from pickers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserPreferences {
    /// The provider ID of the last model selected from the model picker.
    /// Format: `{provider_name}/{model}` (e.g., `"ollama/llama3"`).
    #[serde(default)]
    pub last_model: Option<String>,
    /// The strategy ID of the last strategy selected from the strategy picker.
    /// Format: strategy name (e.g., `"sliding_window"`).
    #[serde(default)]
    pub last_strategy: Option<String>,
    /// Maximum number of lines to display for tool entries in the chat log.
    /// `None` means use the built-in default (5 lines).
    #[serde(default)]
    pub tool_entry_max_lines: Option<u16>,
    /// Minimum number of contiguous excluded entries required to collapse into
    /// a summary line. `None` means use the built-in default (3).
    #[serde(default)]
    pub min_collapse_count: Option<usize>,
    /// The name of the active theme. `None` or `"default"` uses the built-in theme.
    /// Corresponds to a file in `~/.config/nullslop/themes/<name>.toml`.
    #[serde(default)]
    pub theme_name: Option<String>,
    /// The name of the active persona. `None` means use the default (`coding-assistant`).
    /// Corresponds to a file in `~/.config/nullslop/personas/<name>.md`.
    #[serde(default)]
    pub persona_name: Option<String>,
    /// Named session lifecycle recipes — paired setup/teardown commands.
    /// The implicit "blank" lifecycle (no commands) is always available and
    /// does not need to be listed here.
    #[serde(default)]
    #[serde(rename = "session_lifecycle")]
    pub session_lifecycles: Vec<SessionLifecycle>,
    /// Sidebar width in columns. None means use the built-in default (30 columns).
    #[serde(default)]
    pub sidebar_width: Option<u16>,
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
    /// Sliding window configuration for the sliding-window context strategy.
    /// New sessions inherit `size` as their default.
    #[serde(default)]
    pub context_sliding_window: ContextSlidingWindowConfig,
    /// Retry configuration for LLM provider requests.
    #[serde(default)]
    pub request_retry: RequestRetryConfig,
    /// Web fetch tool configuration.
    #[serde(default)]
    pub web_fetch: WebFetchConfig,
    /// OpenRouter web search server tool configuration.
    #[serde(default)]
    pub openrouter_web_search: OpenrouterWebSearchConfig,
}

/// Returns the path to the user preferences file.
///
/// Uses `dirs::config_dir()` → `~/.config/nullslop/nullslop.toml`.
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
pub(crate) fn load_preferences_from<P>(
    path: P,
) -> Result<UserPreferences, Report<UserPreferencesError>>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();

    if !path.exists() {
        return Ok(UserPreferences::default());
    }

    let content = std::fs::read_to_string(path)
        .change_context(UserPreferencesError::Io)
        .attach("failed to read user preferences")?;

    toml::from_str(&content)
        .change_context(UserPreferencesError::Parse)
        .attach("failed to parse user preferences")
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
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent)
            .change_context(UserPreferencesError::Io)
            .attach("failed to create preferences directory")?;
    }

    let content = toml::to_string_pretty(prefs)
        .change_context(UserPreferencesError::Parse)
        .attach("failed to serialize user preferences")?;

    std::fs::write(path.as_ref(), content)
        .change_context(UserPreferencesError::Io)
        .attach("failed to write user preferences")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use tempfile::TempDir;

    use super::*;

    #[rstest::rstest]
    fn default_preferences_has_no_last_model() {
        // Given default preferences.
        let prefs = UserPreferences::default();

        // Then last_model, last_strategy, tool_entry_max_lines, and min_collapse_count are None.
        assert!(prefs.last_model.is_none());
        assert!(prefs.last_strategy.is_none());
        assert!(prefs.tool_entry_max_lines.is_none());
        assert!(prefs.min_collapse_count.is_none());
    }

    #[rstest::rstest]
    fn load_returns_default_when_file_missing() {
        // Given a path to a nonexistent file.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then defaults are returned.
        assert!(prefs.last_model.is_none());
        assert!(prefs.last_strategy.is_none());
        assert!(prefs.tool_entry_max_lines.is_none());
    }

    #[rstest::rstest]
    fn save_then_load_round_trips() {
        // Given preferences with a last_model and last_strategy.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            last_model: Some("ollama/llama3".to_owned()),
            last_strategy: Some("sliding_window".to_owned()),
            tool_entry_max_lines: None,
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![],
            sidebar_width: None,
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            context_sliding_window: ContextSlidingWindowConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
        };

        // When saving and reloading.
        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");

        // Then the round-tripped data matches.
        assert_eq!(reloaded.last_model.as_deref(), Some("ollama/llama3"));
        assert_eq!(reloaded.last_strategy.as_deref(), Some("sliding_window"));
    }

    #[rstest::rstest]
    fn load_parses_toml_content() {
        // Given a TOML file with last_model and last_strategy.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"last_model = "openrouter/anthropic/claude-sonnet-4-20250514"
last_strategy = "sliding_window""#,
        )
        .expect("write");

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then last_model and last_strategy are parsed.
        assert_eq!(
            prefs.last_model.as_deref(),
            Some("openrouter/anthropic/claude-sonnet-4-20250514")
        );
        assert_eq!(prefs.last_strategy.as_deref(), Some("sliding_window"));
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
        assert!(prefs.last_model.is_none());
        assert!(prefs.last_strategy.is_none());
        assert!(prefs.tool_entry_max_lines.is_none());
    }

    #[rstest::rstest]
    fn save_creates_parent_directories() {
        // Given a nested path that doesn't exist.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("nested").join("dir").join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            last_model: Some("test/model".to_owned()),
            last_strategy: None,
            tool_entry_max_lines: None,
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![],
            sidebar_width: None,
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            context_sliding_window: ContextSlidingWindowConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
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
            last_model: None,
            last_strategy: None,
            tool_entry_max_lines: Some(10),
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![],
            sidebar_width: None,
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            context_sliding_window: ContextSlidingWindowConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
        };

        // When saving and reloading.
        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");

        // Then the round-tripped value matches.
        assert_eq!(reloaded.tool_entry_max_lines, Some(10));
    }

    #[rstest::rstest]
    fn save_then_load_round_trips_session_lifecycles() {
        // Given preferences with a session lifecycle.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            last_model: None,
            last_strategy: None,
            tool_entry_max_lines: None,
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![SessionLifecycle {
                name: "fossil branch".to_owned(),
                description: Some("Open a fossil branch in a new workdir".to_owned()),
                setup: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "~/.config/nullslop/scripts/fossil-branch.sh $1".to_owned(),
                    ),
                ),
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "~/.config/nullslop/scripts/fossil-cleanup.sh $1".to_owned(),
                    ),
                ),
            }],
            sidebar_width: None,
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            context_sliding_window: ContextSlidingWindowConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
        };

        // When saving and reloading.
        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");

        // Then the lifecycle is preserved.
        assert_eq!(reloaded.session_lifecycles.len(), 1);
        assert_eq!(reloaded.session_lifecycles[0].name, "fossil branch");
        assert!(matches!(
            reloaded.session_lifecycles[0].setup,
            Some(crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(ref s)) if s == "~/.config/nullslop/scripts/fossil-branch.sh $1"
        ));
    }

    #[rstest::rstest]
    fn default_preferences_has_empty_lifecycles() {
        // Given default preferences.
        let prefs = UserPreferences::default();

        // Then session_lifecycles is empty.
        assert!(prefs.session_lifecycles.is_empty());
    }

    #[rstest::rstest]
    fn save_then_load_round_trips_sidebar_width() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            last_model: None,
            last_strategy: None,
            tool_entry_max_lines: None,
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![],
            sidebar_width: Some(25),
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            context_sliding_window: ContextSlidingWindowConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
        };
        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");
        assert_eq!(reloaded.sidebar_width, Some(25));
    }

    #[rstest::rstest]
    fn preferences_path_ends_with_nullslop_toml() {
        // Given the standard preferences path.
        let path = preferences_path();

        // Then it ends with nullslop/nullslop.toml.
        assert!(path.to_string_lossy().ends_with("nullslop/nullslop.toml"));
    }

    #[rstest::rstest]
    fn load_parses_table_array_session_lifecycle() {
        // Given a TOML file using [[session_lifecycle]] table array syntax.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"last_model = "ollama/llama3"

[[session_lifecycle]]
name = "fossil branch"
description = "Open a fossil branch in a new workdir"
setup_command = "~/.config/nullslop/scripts/fossil-branch.sh $1"
teardown_command = "~/.config/nullslop/scripts/fossil-cleanup.sh $1"
"#,
        )
        .expect("write");

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then session_lifecycles is populated.
        assert_eq!(prefs.session_lifecycles.len(), 1);
        assert_eq!(prefs.session_lifecycles[0].name, "fossil branch");
        assert!(matches!(
            prefs.session_lifecycles[0].setup,
            Some(crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(ref s)) if s == "~/.config/nullslop/scripts/fossil-branch.sh $1"
        ));
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

    // --- S-Tier: Kill mutants for load_preferences / save_preferences ---

    #[rstest::rstest]
    fn load_preferences_actually_reads_file_content() {
        // Kills: replace load_preferences with Ok(Default::default()).
        // If load_preferences were a no-op returning defaults, this would fail
        // because we verify that file content is actually read and parsed.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "last_model = \"openrouter/anthropic/claude-sonnet-4-20250514\"\n\
             last_strategy = \"sliding_window\"\n\
             sidebar_width = 42\n",
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");

        // Then the loaded prefs are NOT defaults — they reflect the file.
        assert_eq!(
            prefs.last_model.as_deref(),
            Some("openrouter/anthropic/claude-sonnet-4-20250514")
        );
        assert_eq!(prefs.last_strategy.as_deref(), Some("sliding_window"));
        assert_eq!(prefs.sidebar_width, Some(42));
    }

    #[rstest::rstest]
    fn save_preferences_actually_writes_to_disk() {
        // Kills: replace save_preferences with Ok(()).
        // If save_preferences were a no-op, the file would not exist on disk.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            last_model: Some("ollama/llama3".to_owned()),
            last_strategy: None,
            tool_entry_max_lines: Some(99),
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![],
            sidebar_width: Some(42),
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            context_sliding_window: ContextSlidingWindowConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
        };

        save_preferences_to(&prefs, &path).expect("save");

        // Then the file exists on disk with the expected content.
        assert!(path.exists(), "save_preferences should create the file");
        let content = std::fs::read_to_string(&path).expect("read back");
        assert!(content.contains("ollama/llama3"));
        assert!(content.contains("42"));
    }

    // --- S-Tier: Kill mutant for RequestRetryConfig::to_retry_config ---

    #[rstest::rstest]
    fn to_retry_config_uses_actual_values_not_defaults() {
        // Kills: replace to_retry_config with Default::default().
        // If to_retry_config returned Default::default(), all durations would be zero.
        let config = RequestRetryConfig {
            max_retries: 3,
            base_delay_secs: 5,
            max_delay_secs: 120,
        };

        let retry = config.to_retry_config();

        assert_eq!(retry.max_retries, 3);
        assert_eq!(retry.base_delay, std::time::Duration::from_secs(5));
        assert_eq!(retry.max_delay, std::time::Duration::from_secs(120));
    }

    // --- WebFetchConfig tests ---

    #[rstest::rstest]
    fn default_web_fetch_config_uses_http_backend() {
        let config = WebFetchConfig::default();
        assert_eq!(config.backend, WebFetchBackend::Http);
    }

    #[rstest::rstest]
    fn default_preferences_has_http_web_fetch() {
        let prefs = UserPreferences::default();
        assert_eq!(prefs.web_fetch.backend, WebFetchBackend::Http);
    }

    #[rstest::rstest]
    fn load_parses_web_fetch_headless_chrome() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"[web_fetch]
backend = "headless-chrome"
"#,
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert_eq!(prefs.web_fetch.backend, WebFetchBackend::HeadlessChrome);
    }

    #[rstest::rstest]
    fn load_rejects_invalid_web_fetch_backend() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"[web_fetch]
backend = "socks"
"#,
        )
        .expect("write");

        let result = load_preferences_from(&path);
        assert!(result.is_err());
    }

    #[rstest::rstest]
    fn save_then_load_round_trips_web_fetch_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            web_fetch: WebFetchConfig {
                backend: WebFetchBackend::HeadlessChrome,
            },
            ..UserPreferences::default()
        };

        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");
        assert_eq!(reloaded.web_fetch.backend, WebFetchBackend::HeadlessChrome);
    }

    // --- OpenRouterWebSearchConfig tests ---

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
    fn save_then_load_round_trips_openrouter_web_search_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            openrouter_web_search: OpenrouterWebSearchConfig {
                engine: Some("exa".to_owned()),
                max_results: Some(10),
                max_total_results: Some(50),
                search_context_size: Some("high".to_owned()),
                allowed_domains: Some(vec!["arxiv.org".to_owned()]),
                excluded_domains: Some(vec!["reddit.com".to_owned()]),
            },
            ..UserPreferences::default()
        };

        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");

        assert_eq!(reloaded.openrouter_web_search.engine.as_deref(), Some("exa"));
        assert_eq!(reloaded.openrouter_web_search.max_results, Some(10));
        assert_eq!(reloaded.openrouter_web_search.max_total_results, Some(50));
        assert_eq!(
            reloaded.openrouter_web_search.search_context_size.as_deref(),
            Some("high")
        );
        assert_eq!(
            reloaded.openrouter_web_search.allowed_domains,
            Some(vec!["arxiv.org".to_owned()])
        );
        assert_eq!(
            reloaded.openrouter_web_search.excluded_domains,
            Some(vec!["reddit.com".to_owned()])
        );
    }

    #[rstest::rstest]
    fn load_parses_openrouter_web_search_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"[openrouter_web_search]
engine = "parallel"
max_results = 5
max_total_results = 20
search_context_size = "medium"
allowed_domains = ["nature.com", "arxiv.org"]
excluded_domains = ["spam.com"]
"#,
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");

        assert_eq!(prefs.openrouter_web_search.engine.as_deref(), Some("parallel"));
        assert_eq!(prefs.openrouter_web_search.max_results, Some(5));
        assert_eq!(prefs.openrouter_web_search.max_total_results, Some(20));
        assert_eq!(
            prefs.openrouter_web_search.search_context_size.as_deref(),
            Some("medium")
        );
        assert_eq!(
            prefs.openrouter_web_search.allowed_domains,
            Some(vec!["nature.com".to_owned(), "arxiv.org".to_owned()])
        );
        assert_eq!(
            prefs.openrouter_web_search.excluded_domains,
            Some(vec!["spam.com".to_owned()])
        );
    }

    #[rstest::rstest]
    fn load_without_openrouter_web_search_section_uses_defaults() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"last_model = "ollama/llama3"
"#,
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");

        let defaults = OpenrouterWebSearchConfig::default();
        assert_eq!(prefs.openrouter_web_search.engine, defaults.engine);
        assert_eq!(prefs.openrouter_web_search.max_results, defaults.max_results);
        assert_eq!(prefs.openrouter_web_search.max_total_results, defaults.max_total_results);
        assert_eq!(
            prefs.openrouter_web_search.search_context_size,
            defaults.search_context_size
        );
        assert_eq!(prefs.openrouter_web_search.allowed_domains, defaults.allowed_domains);
        assert_eq!(prefs.openrouter_web_search.excluded_domains, defaults.excluded_domains);
    }
}
