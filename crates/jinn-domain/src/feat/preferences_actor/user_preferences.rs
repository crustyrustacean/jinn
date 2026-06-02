//! User preferences data type and file I/O.
//!
//! Defines [`UserPreferences`] as the schema for `jinn.toml`,
//! along with loading and saving logic. The file lives at
//! `~/.config/jinn/jinn.toml` and is auto-created on first save.

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

/// A named session lifecycle recipe - paired setup and teardown commands.
///
/// Defined in `jinn.toml` under `[[session_lifecycle]]`. The setup command
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

/// CWD selector configuration.
///
/// Serialized as `[cwd_selector]` in `jinn.toml`.
/// Controls the shell command used to select a new working directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CwdSelectorConfig {
    /// Shell command template. `{path}` is replaced with the search root.
    /// Default: `find -L {path} -type d 2>/dev/null | fzf --no-multi`
    #[serde(default = "CwdSelectorConfig::default_command")]
    pub command: String,
}

impl CwdSelectorConfig {
    /// Returns the default picker command.
    fn default_command() -> String {
        "find -L {path} -type d 2>/dev/null | fzf --no-multi".to_owned()
    }
}

impl Default for CwdSelectorConfig {
    fn default() -> Self {
        Self {
            command: Self::default_command(),
        }
    }
}

/// Default maximum token count for minimap color banding.
const DEFAULT_MINIMAP_MAX_TOKENS: u32 = 2000;

/// Minimap configuration.
///
/// Serialized as `[minimap]` in `jinn.toml`.
/// Controls the token-count range used for the vertical minimap color gradient.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinimapConfig {
    /// Maximum token count for the top band of the minimap gradient.
    /// Entries with more tokens than this get the last band color.
    /// Default: 2000.
    #[serde(default = "default_minimap_max_tokens")]
    pub max_tokens: u32,
}

fn default_minimap_max_tokens() -> u32 {
    DEFAULT_MINIMAP_MAX_TOKENS
}

impl Default for MinimapConfig {
    fn default() -> Self {
        Self {
            max_tokens: DEFAULT_MINIMAP_MAX_TOKENS,
        }
    }
}


/// Default enabled state for read-edit auto-prune.
const DEFAULT_READ_EDIT_ENABLED: bool = true;

/// Default enabled state for todo auto-prune.
const DEFAULT_TODO_ENABLED: bool = true;

/// Read-edit auto-prune configuration.
///
/// Serialized as `[auto_prune.read_edit]` in `jinn.toml`.
/// Controls the auto-prune worker that excludes stale read tool calls and results
/// after the file has been edited twice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadEditAutoPruneConfig {
    #[serde(default = "default_read_edit_enabled")]
    pub enabled: bool,
}

fn default_read_edit_enabled() -> bool {
    DEFAULT_READ_EDIT_ENABLED
}



impl Default for ReadEditAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_READ_EDIT_ENABLED,
        }
    }
}

/// Todo auto-prune configuration.
///
/// Serialized as `[auto_prune.todo]` in `jinn.toml`.
/// Controls the auto-prune worker that excludes stale todo tool call+result
/// pairs, keeping only the most recent one for each tool name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoAutoPruneConfig {
    /// Whether the todo auto-prune worker is active.
    /// Default: `true`.
    #[serde(default = "default_todo_enabled")]
    pub enabled: bool,
}

fn default_todo_enabled() -> bool {
    DEFAULT_TODO_ENABLED
}

impl Default for TodoAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_TODO_ENABLED,
        }
    }
}

/// Default minimum number of in-context entries after a failed edit before pruning.
const DEFAULT_BROKEN_EDIT_MIN_TAIL_ENTRIES: usize = 10;

/// Default enabled state for broken-edit auto-prune.
const DEFAULT_BROKEN_EDIT_ENABLED: bool = true;

/// Broken-edit auto-prune configuration.
///
/// Serialized as `[auto_prune.broken_edit]` in `jinn.toml`.
/// Controls the auto-prune worker that excludes failed edit tool call+result pairs
/// from the LLM context once enough conversation has moved on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokenEditAutoPruneConfig {
    /// Whether the broken-edit auto-prune worker is active.
    /// Default: `true`.
    #[serde(default = "default_broken_edit_enabled")]
    pub enabled: bool,
    /// Minimum number of in-context entries that must appear after the failed edit
    /// ToolCall before the call+result pair is pruned.
    /// Default: 10.
    #[serde(default = "default_broken_edit_min_tail_entries")]
    pub min_tail_entries: usize,
}

fn default_broken_edit_enabled() -> bool {
    DEFAULT_BROKEN_EDIT_ENABLED
}

fn default_broken_edit_min_tail_entries() -> usize {
    DEFAULT_BROKEN_EDIT_MIN_TAIL_ENTRIES
}

impl Default for BrokenEditAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_BROKEN_EDIT_ENABLED,
            min_tail_entries: DEFAULT_BROKEN_EDIT_MIN_TAIL_ENTRIES,
        }
    }
}

/// Default max file edits for double-edit auto-prune.
const DEFAULT_DOUBLE_EDIT_MAX_FILE_EDITS: usize = 2;

/// Default enabled state for double-edit auto-prune.
const DEFAULT_DOUBLE_EDIT_ENABLED: bool = true;

/// Double-edit auto-prune configuration.
///
/// Serialized as `[auto_prune.double_edit]` in `jinn.toml`.
/// Controls the auto-prune worker that caps the number of edit/write
/// tool call+result pairs per file path, keeping only the most recent ones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoubleEditAutoPruneConfig {
    /// Whether the double-edit auto-prune worker is active.
    /// Default: `true`.
    #[serde(default = "default_double_edit_enabled")]
    pub enabled: bool,
    /// Maximum number of edit/write tool call+result pairs to keep per file path.
    /// Oldest pairs are pruned when this limit is exceeded.
    /// Set to 0 to disable pruning (no limit).
    /// Default: 2.
    #[serde(default = "default_double_edit_max_file_edits")]
    pub max_file_edits: usize,
}

fn default_double_edit_enabled() -> bool {
    DEFAULT_DOUBLE_EDIT_ENABLED
}

fn default_double_edit_max_file_edits() -> usize {
    DEFAULT_DOUBLE_EDIT_MAX_FILE_EDITS
}

impl Default for DoubleEditAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_DOUBLE_EDIT_ENABLED,
            max_file_edits: DEFAULT_DOUBLE_EDIT_MAX_FILE_EDITS,
        }
    }
}


/// Default number of consecutive read pairs to keep per file path.
const DEFAULT_CONSECUTIVE_READS_KEEP_LAST: usize = 3;

/// Default enabled state for consecutive-reads auto-prune.
const DEFAULT_CONSECUTIVE_READS_ENABLED: bool = true;

/// Consecutive-reads auto-prune configuration.
///
/// Serialized as `[auto_prune.consecutive_reads]` in `jinn.toml`.
/// Controls the auto-prune worker that caps the number of `read`
/// tool call+result pairs per file path, keeping only the most recent ones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsecutiveReadsAutoPruneConfig {
    /// Whether the consecutive-reads auto-prune worker is active.
    /// Default: `true`.
    #[serde(default = "default_consecutive_reads_enabled")]
    pub enabled: bool,
    /// Number of most recent `read` tool call+result pairs to keep per file path.
    /// Older pairs are pruned when this limit is exceeded.
    /// Minimum 1 (clamped during worker construction).
    /// Default: 3.
    #[serde(default = "default_consecutive_reads_keep_last")]
    pub keep_last: usize,
}

fn default_consecutive_reads_enabled() -> bool {
    DEFAULT_CONSECUTIVE_READS_ENABLED
}

fn default_consecutive_reads_keep_last() -> usize {
    DEFAULT_CONSECUTIVE_READS_KEEP_LAST
}

impl Default for ConsecutiveReadsAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_CONSECUTIVE_READS_ENABLED,
            keep_last: DEFAULT_CONSECUTIVE_READS_KEEP_LAST,
        }
    }
}
/// Default regex prune rule tool name.
const DEFAULT_REGEX_TOOL_NAME: &str = "bash";

/// Default regex prune rule keep_last.
const DEFAULT_REGEX_KEEP_LAST: usize = 1;

/// Default enabled state for regex auto-prune.
const DEFAULT_REGEX_ENABLED: bool = true;

/// A single regex-based auto-prune rule.
///
/// Serialized as `[[auto_prune.regex]]` in `jinn.toml`.
/// Each rule matches tool calls by name and content, keeping only the
/// most recent `keep_last` matching call+result pairs in context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegexPruneRule {
    /// Regex pattern to match against the tool call's text output.
    /// The regex is tested against `"{name}: {arguments}"`.
    pub pattern: String,
    /// Tool name to filter by. Only tool calls with this name are considered.
    /// Default: `"bash"`.
    #[serde(default = "default_regex_tool_name")]
    pub tool_name: String,
    /// Number of most recent matching pairs to keep in context.
    /// Minimum 1 (clamped at worker construction).
    /// Default: 1.
    #[serde(default = "default_regex_keep_last")]
    pub keep_last: usize,
}

fn default_regex_tool_name() -> String {
    DEFAULT_REGEX_TOOL_NAME.to_owned()
}

fn default_regex_keep_last() -> usize {
    DEFAULT_REGEX_KEEP_LAST
}

/// Regex-based auto-prune configuration.
///
/// Serialized as `[auto_prune.regex]` in `jinn.toml`.
/// Contains a list of regex rules that identify tool calls to prune.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegexAutoPruneConfig {
    /// Whether the regex auto-prune worker is active.
    /// Default: `true`.
    #[serde(default = "default_regex_enabled")]
    pub enabled: bool,
    /// List of regex prune rules.
    /// Default: empty (no rules).
    #[serde(default)]
    pub rules: Vec<RegexPruneRule>,
}

fn default_regex_enabled() -> bool {
    DEFAULT_REGEX_ENABLED
}

impl Default for RegexAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_REGEX_ENABLED,
            rules: Vec::new(),
        }
    }
}

/// Auto-prune configuration.
///
/// Serialized as `[auto_prune]` in `jinn.toml`.
/// Groups all auto-prune strategy configurations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoPruneConfig {
    /// Read-edit auto-prune strategy configuration.
    #[serde(default)]
    pub read_edit: ReadEditAutoPruneConfig,
    /// Regex-based auto-prune strategy configuration.
    #[serde(default)]
    pub regex: RegexAutoPruneConfig,
    /// Broken-edit auto-prune strategy configuration.
    #[serde(default)]
    pub broken_edit: BrokenEditAutoPruneConfig,
    /// Todo auto-prune strategy configuration.
    #[serde(default)]
    pub todo: TodoAutoPruneConfig,
    /// Double-edit auto-prune strategy configuration.
    #[serde(default)]
    pub double_edit: DoubleEditAutoPruneConfig,
    /// Consecutive-reads auto-prune strategy configuration.
    #[serde(default)]
    pub consecutive_reads: ConsecutiveReadsAutoPruneConfig,
}

impl Default for AutoPruneConfig {
    fn default() -> Self {
        Self {
            read_edit: ReadEditAutoPruneConfig::default(),
            regex: RegexAutoPruneConfig::default(),
            broken_edit: BrokenEditAutoPruneConfig::default(),
            todo: TodoAutoPruneConfig::default(),
            double_edit: DoubleEditAutoPruneConfig::default(),
            consecutive_reads: ConsecutiveReadsAutoPruneConfig::default(),
        }
    }
}

/// Default token threshold for auto-compaction.
const DEFAULT_COMPACTION_THRESHOLD: f64 = 0.7;

/// Default number of recent tokens to reserve from compaction.
const DEFAULT_RESERVE_TOKENS: usize = 20_000;

/// Default fallback context window when the provider doesn't report one.
const DEFAULT_FALLBACK_CONTEXT_WINDOW: usize = 150_000;

/// Compaction configuration.
///
/// Serialized as `[compaction]` in `jinn.toml`.
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
/// Serialized as `[context_sliding_window]` in `jinn.toml`.
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
/// Selected once at startup from `jinn.toml` and never changes at runtime.
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
/// Serialized as `[web_fetch]` in `jinn.toml`.
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
/// Serialized as `[openrouter_web_search]` in `jinn.toml`.
/// Controls parameters sent to the `openrouter:web_search` server tool.
/// All fields are optional - when `None`, the parameter is omitted from
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
/// Serialized as `[request_retry]` in `jinn.toml`.
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
    /// Convert to the provider-crate [`jinn_provider::RetryConfig`].
    #[must_use]
    pub fn to_retry_config(&self) -> jinn_provider::RetryConfig {
        jinn_provider::RetryConfig {
            max_retries: self.max_retries,
            base_delay: std::time::Duration::from_secs(self.base_delay_secs),
            max_delay: std::time::Duration::from_secs(self.max_delay_secs),
        }
    }
}

/// User preferences persisted in `jinn.toml`.
///
/// This file stores user behavior preferences that should survive
/// app restarts - e.g., the last model and strategy selected from pickers.
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
    /// Corresponds to a file in `~/.config/jinn/themes/<name>.toml`.
    #[serde(default)]
    pub theme_name: Option<String>,
    /// The name of the active persona. `None` means use the default (`coding-assistant`).
    /// Corresponds to a file in `~/.config/jinn/personas/<name>.md`.
    #[serde(default)]
    pub persona_name: Option<String>,
    /// Named session lifecycle recipes - paired setup/teardown commands.
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
    /// CWD selector configuration.
    #[serde(default)]
    pub cwd_selector: CwdSelectorConfig,
    /// Minimap configuration.
    #[serde(default)]
    pub minimap: MinimapConfig,
    /// Auto-prune configuration.
    #[serde(default)]
    pub auto_prune: AutoPruneConfig,
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
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
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
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
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
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
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
                        "~/.config/jinn/scripts/fossil-branch.sh $1".to_owned(),
                    ),
                ),
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "~/.config/jinn/scripts/fossil-cleanup.sh $1".to_owned(),
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
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
        };

        // When saving and reloading.
        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");

        // Then the lifecycle is preserved.
        assert_eq!(reloaded.session_lifecycles.len(), 1);
        assert_eq!(reloaded.session_lifecycles[0].name, "fossil branch");
        assert!(matches!(
            reloaded.session_lifecycles[0].setup,
            Some(crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(ref s)) if s == "~/.config/jinn/scripts/fossil-branch.sh $1"
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
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
        };
        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");
        assert_eq!(reloaded.sidebar_width, Some(25));
    }

    #[rstest::rstest]
    fn preferences_path_ends_with_jinn_toml() {
        // Given the standard preferences path.
        let path = preferences_path();

        // Then it ends with jinn/jinn.toml.
        assert!(path.to_string_lossy().ends_with("jinn/jinn.toml"));
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
setup_command = "~/.config/jinn/scripts/fossil-branch.sh $1"
teardown_command = "~/.config/jinn/scripts/fossil-cleanup.sh $1"
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
            Some(crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(ref s)) if s == "~/.config/jinn/scripts/fossil-branch.sh $1"
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

        // Then the loaded prefs are NOT defaults - they reflect the file.
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
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
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

        assert_eq!(
            reloaded.openrouter_web_search.engine.as_deref(),
            Some("exa")
        );
        assert_eq!(reloaded.openrouter_web_search.max_results, Some(10));
        assert_eq!(reloaded.openrouter_web_search.max_total_results, Some(50));
        assert_eq!(
            reloaded
                .openrouter_web_search
                .search_context_size
                .as_deref(),
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

        assert_eq!(
            prefs.openrouter_web_search.engine.as_deref(),
            Some("parallel")
        );
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
        assert_eq!(
            prefs.openrouter_web_search.max_results,
            defaults.max_results
        );
        assert_eq!(
            prefs.openrouter_web_search.max_total_results,
            defaults.max_total_results
        );
        assert_eq!(
            prefs.openrouter_web_search.search_context_size,
            defaults.search_context_size
        );
        assert_eq!(
            prefs.openrouter_web_search.allowed_domains,
            defaults.allowed_domains
        );
        assert_eq!(
            prefs.openrouter_web_search.excluded_domains,
            defaults.excluded_domains
        );
    }

    // --- MinimapConfig tests ---

    #[rstest::rstest]
    fn default_minimap_config_has_max_tokens_2000() {
        // Given default minimap config.
        let config = MinimapConfig::default();

        // Then max_tokens is 2000.
        assert_eq!(config.max_tokens, 2000);
    }

    #[rstest::rstest]
    fn default_preferences_has_default_minimap_config() {
        // Given default preferences.
        let prefs = UserPreferences::default();

        // Then minimap config uses defaults.
        assert_eq!(prefs.minimap.max_tokens, 2000);
    }

    #[rstest::rstest]
    fn load_parses_minimap_config() {
        // Given a TOML file with a minimap section.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"[minimap]
max_tokens = 5000
"#,
        )
        .expect("write");

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then minimap config is parsed.
        assert_eq!(prefs.minimap.max_tokens, 5000);
    }

    #[rstest::rstest]
    fn save_then_load_round_trips_minimap_config() {
        // Given preferences with a custom minimap config.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            minimap: MinimapConfig { max_tokens: 5000 },
            ..UserPreferences::default()
        };

        // When saving and reloading.
        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");

        // Then the round-tripped value matches.
        assert_eq!(reloaded.minimap.max_tokens, 5000);
    }

    #[rstest::rstest]
    fn load_without_minimap_section_uses_defaults() {
        // Given a TOML file without a minimap section.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"last_model = "ollama/llama3"
"#,
        )
        .expect("write");

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then minimap uses defaults.
        assert_eq!(prefs.minimap.max_tokens, 2000);
    }

    // --- AutoPruneConfig tests ---

    #[rstest::rstest]
    fn default_auto_prune_config_has_defaults() {
        let config = AutoPruneConfig::default();
        assert!(config.read_edit.enabled);
        assert!(config.todo.enabled);
        assert!(config.consecutive_reads.enabled);
        assert_eq!(config.consecutive_reads.keep_last, 3);
    }

    #[rstest::rstest]
    fn default_preferences_has_default_auto_prune_config() {
        let prefs = UserPreferences::default();
        assert!(prefs.auto_prune.read_edit.enabled);
        assert!(prefs.auto_prune.todo.enabled);
        assert!(prefs.auto_prune.consecutive_reads.enabled);
    }

    #[rstest::rstest]
    fn load_parses_auto_prune_read_edit_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"[auto_prune.read_edit]
enabled = false
"#,
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(!prefs.auto_prune.read_edit.enabled);
    }

    #[rstest::rstest]
    fn load_parses_auto_prune_consecutive_reads_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"[auto_prune.consecutive_reads]
enabled = false
keep_last = 5
"#,
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(!prefs.auto_prune.consecutive_reads.enabled);
        assert_eq!(prefs.auto_prune.consecutive_reads.keep_last, 5);
    }

    #[rstest::rstest]
    fn save_then_load_round_trips_auto_prune_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            auto_prune: AutoPruneConfig {
                read_edit: ReadEditAutoPruneConfig {
                    enabled: false,
                },
                regex: RegexAutoPruneConfig::default(),
                broken_edit: BrokenEditAutoPruneConfig {
                    enabled: false,
                    min_tail_entries: 3,
                },
                todo: TodoAutoPruneConfig { enabled: false },
                double_edit: DoubleEditAutoPruneConfig::default(),
                consecutive_reads: ConsecutiveReadsAutoPruneConfig::default(),
            },
            ..UserPreferences::default()
        };

        save_preferences_to(&prefs, &path).expect("save");

        let reloaded = load_preferences_from(&path).expect("load");
        assert!(!reloaded.auto_prune.read_edit.enabled);
        assert!(!reloaded.auto_prune.broken_edit.enabled);
        assert_eq!(reloaded.auto_prune.broken_edit.min_tail_entries, 3);
        assert!(!reloaded.auto_prune.todo.enabled);
    }

    #[rstest::rstest]
    fn load_without_auto_prune_section_uses_defaults() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r##"last_model = 'ollama/llama3'"##,
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(prefs.auto_prune.read_edit.enabled);
        assert!(prefs.auto_prune.todo.enabled);
    }

    #[rstest::rstest]
    fn load_parses_auto_prune_todo_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"[auto_prune.todo]
enabled = false
"#,
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(!prefs.auto_prune.todo.enabled);
        // read_edit should still have defaults
        assert!(prefs.auto_prune.read_edit.enabled);
    }

    // --- RegexAutoPruneConfig tests ---

    #[rstest::rstest]
    fn default_regex_config_is_empty_rules_and_enabled() {
        let config = RegexAutoPruneConfig::default();
        assert!(config.enabled);
        assert!(config.rules.is_empty());
    }

    #[rstest::rstest]
    fn regex_prune_rule_defaults_to_bash_tool_name() {
        let rule = RegexPruneRule {
            pattern: "cargo check".to_owned(),
            tool_name: default_regex_tool_name(),
            keep_last: default_regex_keep_last(),
        };
        assert_eq!(rule.tool_name, "bash");
        assert_eq!(rule.keep_last, 1);
    }

    #[rstest::rstest]
    fn save_then_load_round_trips_regex_prune_rules() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            auto_prune: AutoPruneConfig {
                regex: RegexAutoPruneConfig {
                    enabled: true,
                    rules: vec![
                        RegexPruneRule {
                            pattern: "cargo check".to_owned(),
                            tool_name: "bash".to_owned(),
                            keep_last: 1,
                        },
                        RegexPruneRule {
                            pattern: "cargo test".to_owned(),
                            tool_name: "bash".to_owned(),
                            keep_last: 2,
                        },
                    ],
                },
                ..AutoPruneConfig::default()
            },
            ..UserPreferences::default()
        };

        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");

        assert_eq!(reloaded.auto_prune.regex.rules.len(), 2);
        assert_eq!(reloaded.auto_prune.regex.rules[0].pattern, "cargo check");
        assert_eq!(reloaded.auto_prune.regex.rules[0].tool_name, "bash");
        assert_eq!(reloaded.auto_prune.regex.rules[0].keep_last, 1);
        assert_eq!(reloaded.auto_prune.regex.rules[1].pattern, "cargo test");
        assert_eq!(reloaded.auto_prune.regex.rules[1].keep_last, 2);
    }

    #[rstest::rstest]
    fn load_parses_multiple_regex_rules() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"[[auto_prune.regex.rules]]
pattern = "cargo check"
tool_name = "bash"
keep_last = 1

[[auto_prune.regex.rules]]
pattern = "cargo test"
tool_name = "bash"
keep_last = 2
"#,
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert_eq!(prefs.auto_prune.regex.rules.len(), 2);
        assert_eq!(prefs.auto_prune.regex.rules[0].pattern, "cargo check");
        assert_eq!(prefs.auto_prune.regex.rules[1].pattern, "cargo test");
    }

    #[rstest::rstest]
    fn load_parses_regex_rules_with_defaults() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"[[auto_prune.regex.rules]]
pattern = "cargo check"
"#,
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert_eq!(prefs.auto_prune.regex.rules.len(), 1);
        assert_eq!(prefs.auto_prune.regex.rules[0].pattern, "cargo check");
        assert_eq!(prefs.auto_prune.regex.rules[0].tool_name, "bash");
        assert_eq!(prefs.auto_prune.regex.rules[0].keep_last, 1);
    }

    #[rstest::rstest]
    fn load_without_auto_prune_regex_section_uses_defaults() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"last_model = "ollama/llama3"
"#,
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(prefs.auto_prune.regex.enabled);
        assert!(prefs.auto_prune.regex.rules.is_empty());
    }

    #[rstest::rstest]
    fn load_parses_regex_rules_with_header_section() {
        // Mirrors the real user config: [auto_prune.regex] header + [[auto_prune.regex.rules]] entries.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"[auto_prune.regex]
enabled = true

[[auto_prune.regex.rules]]
pattern = "ls"
tool_name = "bash"
keep_last = 1

[[auto_prune.regex.rules]]
pattern = "cargo check"
tool_name = "bash"
keep_last = 1
"#,
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(prefs.auto_prune.regex.enabled);
        assert_eq!(prefs.auto_prune.regex.rules.len(), 2);
        assert_eq!(prefs.auto_prune.regex.rules[0].pattern, "ls");
        assert_eq!(prefs.auto_prune.regex.rules[1].pattern, "cargo check");
    }

    #[rstest::rstest]
    fn serialize_regex_rules_produces_correct_toml() {
        let prefs = UserPreferences {
            auto_prune: AutoPruneConfig {
                regex: RegexAutoPruneConfig {
                    enabled: true,
                    rules: vec![
                        RegexPruneRule {
                            pattern: "ls".to_owned(),
                            tool_name: "bash".to_owned(),
                            keep_last: 1,
                        },
                        RegexPruneRule {
                            pattern: "cargo check".to_owned(),
                            tool_name: "bash".to_owned(),
                            keep_last: 1,
                        },
                    ],
                },
                ..AutoPruneConfig::default()
            },
            ..UserPreferences::default()
        };

        let toml_str = toml::to_string_pretty(&prefs).expect("serialize");
        eprintln!("SERIALIZED TOML:\n{toml_str}");

        // Round-trip back
        let reloaded: UserPreferences = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(reloaded.auto_prune.regex.rules.len(), 2);
        assert_eq!(reloaded.auto_prune.regex.rules[0].pattern, "ls");
        assert_eq!(reloaded.auto_prune.regex.rules[1].pattern, "cargo check");
    }
}
