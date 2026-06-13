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

/// Canonical default `jinn.toml` embedded at compile time.
///
/// Used both to auto-create the file on first run and to back the
/// `jinn config init` subcommand. A round-trip equality test in this
/// module's test suite asserts that this string deserializes to
/// exactly `UserPreferences::default()`, which is the CI gate that
/// prevents the shipped template from drifting from the struct.
pub(crate) const DEFAULT_CONFIG: &str = include_str!("default_jinn.toml");

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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Default enabled state for edit-read auto-prune.
const DEFAULT_EDIT_READ_ENABLED: bool = true;

/// Default `min_age` for edit-read auto-prune.
///
/// Number of entries from the end of history within which prior
/// edit/write call+result pairs are protected from pruning
/// when a same-file read occurs.
const DEFAULT_EDIT_READ_MIN_AGE: usize = 50;

/// Edit-read auto-prune configuration.
///
/// Serialized as `[auto_prune.edit_read]` in `jinn.toml`.
/// Controls the auto-prune worker that excludes stale edit/write tool calls
/// when a same-file read follows, since the read output represents the
/// current file state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditReadAutoPruneConfig {
    #[serde(default = "default_edit_read_enabled")]
    pub enabled: bool,
    /// Minimum number of entries from the end of history that must
    /// appear after an edit/write call before it may be pruned when
    /// a same-file read follows. Counts every entry, regardless of
    /// in-context status. Set to 0 to disable protection.
    /// Default: 50.
    #[serde(default = "default_edit_read_min_age")]
    pub min_age: usize,
}

fn default_edit_read_enabled() -> bool {
    DEFAULT_EDIT_READ_ENABLED
}

fn default_edit_read_min_age() -> usize {
    DEFAULT_EDIT_READ_MIN_AGE
}

impl Default for EditReadAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_EDIT_READ_ENABLED,
            min_age: DEFAULT_EDIT_READ_MIN_AGE,
        }
    }
}

/// Default enabled state for read-edit auto-prune.
const DEFAULT_READ_EDIT_ENABLED: bool = true;

/// Default `min_age` for read-edit auto-prune.
///
/// Number of entries from the end of history within which
/// read call+result pairs are protected from pruning.
const DEFAULT_READ_EDIT_MIN_AGE: usize = 50;

/// Default threshold for read-edit auto-prune.
///
/// Number of edit/write operations on the same file required before
/// pruning the prior read call+result pair.
const DEFAULT_READ_EDIT_THRESHOLD: usize = 2;

/// Read-edit auto-prune configuration.
///
/// Serialized as `[auto_prune.read_edit]` in `jinn.toml`.
/// Controls the auto-prune worker that excludes stale read tool calls
/// after the file has been edited a configurable number of times.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadEditAutoPruneConfig {
    #[serde(default = "default_read_edit_enabled")]
    pub enabled: bool,
    /// Minimum number of entries from the end of history within which
    /// read call+result pairs are protected from pruning.
    /// Counts every entry, regardless of in-context status.
    /// Set to 0 to disable protection.
    /// Default: 50.
    #[serde(default = "default_read_edit_min_age")]
    pub min_age: usize,
    /// Number of edit/write operations on the same file required before
    /// pruning the prior read call+result pair.
    /// Default: 2.
    #[serde(default = "default_read_edit_threshold")]
    pub threshold: usize,
}

fn default_read_edit_enabled() -> bool {
    DEFAULT_READ_EDIT_ENABLED
}

fn default_read_edit_min_age() -> usize {
    DEFAULT_READ_EDIT_MIN_AGE
}

fn default_read_edit_threshold() -> usize {
    DEFAULT_READ_EDIT_THRESHOLD
}

impl Default for ReadEditAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_READ_EDIT_ENABLED,
            min_age: DEFAULT_READ_EDIT_MIN_AGE,
            threshold: DEFAULT_READ_EDIT_THRESHOLD,
        }
    }
}

/// Default enabled state for todo auto-prune.
const DEFAULT_TODO_ENABLED: bool = true;

/// Default `min_age` for todo auto-prune.
///
/// Number of entries from the end of history that must
/// appear after a todo tool call before pruning may exclude the
/// call+result pair.
const DEFAULT_TODO_MIN_AGE: usize = 50;
/// pairs, keeping only the most recent one for each tool name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TodoAutoPruneConfig {
    /// Whether the todo auto-prune worker is active.
    /// Default: `true`.
    #[serde(default = "default_todo_enabled")]
    pub enabled: bool,
    /// Minimum number of entries from the end of history that must
    /// appear after a todo tool call before pruning may exclude the
    /// call+result pair. Counts every entry, regardless of in-context
    /// status. Set to 0 to disable protection.
    /// Default: 50.
    #[serde(default = "default_todo_min_age")]
    pub min_age: usize,
}

fn default_todo_enabled() -> bool {
    DEFAULT_TODO_ENABLED
}

fn default_todo_min_age() -> usize {
    DEFAULT_TODO_MIN_AGE
}

impl Default for TodoAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_TODO_ENABLED,
            min_age: DEFAULT_TODO_MIN_AGE,
        }
    }
}

/// Default minimum age for broken-edit auto-prune.
const DEFAULT_BROKEN_EDIT_MIN_AGE: usize = 10;

/// Default enabled state for broken-edit auto-prune.
const DEFAULT_BROKEN_EDIT_ENABLED: bool = true;

/// Broken-edit auto-prune configuration.
///
/// Serialized as `[auto_prune.broken_edit]` in `jinn.toml`.
/// Controls the auto-prune worker that excludes failed edit tool call+result pairs
/// from the LLM context once enough conversation has moved on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrokenEditAutoPruneConfig {
    /// Whether the broken-edit auto-prune worker is active.
    /// Default: `true`.
    #[serde(default = "default_broken_edit_enabled")]
    pub enabled: bool,
    /// Minimum number of entries from the end of history that must
    /// appear after the failed edit ToolCall before the call+result
    /// pair may be pruned. Counts every entry, regardless of in-context
    /// status. Set to 0 to disable protection.
    /// Default: 10.
    #[serde(default = "default_broken_edit_min_age", alias = "min_tail_entries")]
    pub min_age: usize,
}

fn default_broken_edit_enabled() -> bool {
    DEFAULT_BROKEN_EDIT_ENABLED
}

fn default_broken_edit_min_age() -> usize {
    DEFAULT_BROKEN_EDIT_MIN_AGE
}

impl Default for BrokenEditAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_BROKEN_EDIT_ENABLED,
            min_age: DEFAULT_BROKEN_EDIT_MIN_AGE,
        }
    }
}

/// Default max file edits for double-edit auto-prune.
const DEFAULT_DOUBLE_EDIT_MAX_FILE_EDITS: usize = 2;

/// Default enabled state for double-edit auto-prune.
const DEFAULT_DOUBLE_EDIT_ENABLED: bool = true;

/// Default `min_age` for double-edit auto-prune.
///
/// Number of entries from the end of history within which edit/write
/// call+result pairs on a file are protected from pruning even when the
/// per-file cap (`max_file_edits`) would otherwise exclude them.
const DEFAULT_DOUBLE_EDIT_MIN_AGE: usize = 20;

/// Double-edit auto-prune configuration.
///
/// Serialized as `[auto_prune.double_edit]` in `jinn.toml`.
/// Controls the auto-prune worker that caps the number of edit/write
/// tool call+result pairs per file path, keeping only the most recent ones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Minimum number of entries from the end of history that must
    /// appear after an edit/write call before it may be pruned.
    /// Counts every entry, regardless of in-context status.
    /// Set to 0 to disable protection (preserves pre-`min_age` behavior).
    /// Default: 20.
    #[serde(default = "default_double_edit_min_age")]
    pub min_age: usize,
}

fn default_double_edit_enabled() -> bool {
    DEFAULT_DOUBLE_EDIT_ENABLED
}

fn default_double_edit_max_file_edits() -> usize {
    DEFAULT_DOUBLE_EDIT_MAX_FILE_EDITS
}

fn default_double_edit_min_age() -> usize {
    DEFAULT_DOUBLE_EDIT_MIN_AGE
}

impl Default for DoubleEditAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_DOUBLE_EDIT_ENABLED,
            max_file_edits: DEFAULT_DOUBLE_EDIT_MAX_FILE_EDITS,
            min_age: DEFAULT_DOUBLE_EDIT_MIN_AGE,
        }
    }
}

/// Default number of consecutive read pairs to keep per file path.
const DEFAULT_CONSECUTIVE_READS_KEEP_LAST: usize = 5;

/// Default enabled state for consecutive-reads auto-prune.
const DEFAULT_CONSECUTIVE_READS_ENABLED: bool = true;

/// Default minimum age for consecutive-reads auto-prune.
const DEFAULT_CONSECUTIVE_READS_MIN_AGE: usize = 80;

/// Consecutive-reads auto-prune configuration.
///
/// Serialized as `[auto_prune.consecutive_reads]` in `jinn.toml`.
/// Controls the auto-prune worker that caps the number of `read`
/// tool call+result pairs per file path, keeping only the most recent ones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Minimum number of entries from the end of history within which
    /// read pairs are protected from pruning even when they would
    /// otherwise be pruned by `keep_last`. Counts every entry, regardless
    /// of in-context status. Set to 0 to disable protection.
    /// Default: `50`.
    #[serde(default = "default_consecutive_reads_min_age")]
    pub min_age: usize,
}

fn default_consecutive_reads_enabled() -> bool {
    DEFAULT_CONSECUTIVE_READS_ENABLED
}

fn default_consecutive_reads_keep_last() -> usize {
    DEFAULT_CONSECUTIVE_READS_KEEP_LAST
}

fn default_consecutive_reads_min_age() -> usize {
    DEFAULT_CONSECUTIVE_READS_MIN_AGE
}

impl Default for ConsecutiveReadsAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_CONSECUTIVE_READS_ENABLED,
            keep_last: DEFAULT_CONSECUTIVE_READS_KEEP_LAST,
            min_age: DEFAULT_CONSECUTIVE_READS_MIN_AGE,
        }
    }
}

/// Default enabled state for tool-age-window auto-prune.
const DEFAULT_TOOL_AGE_WINDOW_ENABLED: bool = true;

/// Default `min_age` for tool-age-window auto-prune.
///
/// Number of entries from the end of history within which `ToolCall`/
/// `ToolResult` pairs are protected from pruning.
const DEFAULT_TOOL_AGE_WINDOW_MIN_AGE: usize = 150;

/// Tool-age-window auto-prune configuration.
///
/// Serialized as `[auto_prune.tool_age_window]` in `jinn.toml`.
/// Controls the auto-prune worker that excludes any `ToolCall`/`ToolResult`
/// pair older than `min_age` entries from the end of history. Both
/// halves of a pair are always excluded together.
///
/// The window counts every entry in raw history regardless of in-context
/// status, so that multiple auto-prune workers compose cleanly: each
/// worker's prune region is fixed by raw history length alone, not by what
/// has already been `ForcedExclude`d by other workers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolAgeWindowAutoPruneConfig {
    /// Whether the tool-age-window auto-prune worker is active.
    /// Default: `true`.
    #[serde(default = "default_tool_age_window_enabled")]
    pub enabled: bool,
    /// Minimum number of entries from the end of history within which
    /// `ToolCall`/`ToolResult` pairs are protected from pruning.
    /// Counts every entry, regardless of in-context status.
    /// Minimum 1 (clamped at worker construction).
    /// Default: 100.
    #[serde(default = "default_tool_age_window_min_age")]
    pub min_age: usize,
}

fn default_tool_age_window_enabled() -> bool {
    DEFAULT_TOOL_AGE_WINDOW_ENABLED
}

fn default_tool_age_window_min_age() -> usize {
    DEFAULT_TOOL_AGE_WINDOW_MIN_AGE
}

impl Default for ToolAgeWindowAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_TOOL_AGE_WINDOW_ENABLED,
            min_age: DEFAULT_TOOL_AGE_WINDOW_MIN_AGE,
        }
    }
}
/// Default enabled state for trivial-assistant auto-prune.
const DEFAULT_TRIVIAL_ASSISTANT_ENABLED: bool = true;

/// Default minimum age for trivial-assistant auto-prune.
const DEFAULT_TRIVIAL_ASSISTANT_MIN_AGE: usize = 100;

/// Default token threshold below which an assistant entry is considered
/// "trivial" (small enough to prune when it lands outside the window).
const DEFAULT_TRIVIAL_ASSISTANT_MAX_TOKENS: usize = 80;

/// Trivial-assistant auto-prune configuration.
///
/// Serialized as `[auto_prune.trivial_assistant]` in `jinn.toml`.
/// Controls the auto-prune worker that excludes any `Assistant` entry that
/// (a) is older than `min_age` entries from the end of history and
/// (b) is at most `max_tokens` tokens long.
///
/// The window counts every entry in raw history regardless of in-context
/// status, so that multiple auto-prune workers compose cleanly: each
/// worker's prune region is fixed by raw history length alone, not by what
/// has already been `ForcedExclude`d by other workers. Tokens are counted
/// via the same tiktoken `o200k_base` encoder used by the token-count
/// actor and minimap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrivialAssistantAutoPruneConfig {
    /// Whether the trivial-assistant auto-prune worker is active.
    /// Default: `true`.
    #[serde(default = "default_trivial_assistant_enabled")]
    pub enabled: bool,
    /// Minimum number of entries from the end of history within which
    /// assistant entries are protected from pruning even when they would
    /// otherwise qualify as trivial. Counts every entry, regardless of
    /// in-context status. Set to 0 to disable protection.
    /// Default: `50`.
    #[serde(
        default = "default_trivial_assistant_min_age",
        alias = "max_age_entries"
    )]
    pub min_age: usize,
    /// Maximum number of tokens (tiktoken `o200k_base`) below which an
    /// `Assistant` entry is considered trivial. Minimum 1 (clamped at
    /// evaluation time).
    /// Default: `80`.
    #[serde(default = "default_trivial_assistant_max_tokens")]
    pub max_tokens: usize,
}

fn default_trivial_assistant_enabled() -> bool {
    DEFAULT_TRIVIAL_ASSISTANT_ENABLED
}

fn default_trivial_assistant_min_age() -> usize {
    DEFAULT_TRIVIAL_ASSISTANT_MIN_AGE
}

fn default_trivial_assistant_max_tokens() -> usize {
    DEFAULT_TRIVIAL_ASSISTANT_MAX_TOKENS
}

impl Default for TrivialAssistantAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_TRIVIAL_ASSISTANT_ENABLED,
            min_age: DEFAULT_TRIVIAL_ASSISTANT_MIN_AGE,
            max_tokens: DEFAULT_TRIVIAL_ASSISTANT_MAX_TOKENS,
        }
    }
}

/// Default enabled state for anchored-assistant auto-prune.
const DEFAULT_ANCHORED_ASSISTANT_ENABLED: bool = true;

/// Default radius (in raw history entries) within which an Assistant entry is
/// protected from pruning regardless of token count.
const DEFAULT_ANCHORED_ASSISTANT_RADIUS: usize = 100;

/// Default minimum age for anchored-assistant auto-prune.
const DEFAULT_ANCHORED_ASSISTANT_MIN_AGE: usize = 50;

/// Anchored-assistant auto-prune strategy configuration.
///
/// Serialized as `[auto_prune.anchored_assistant]` in `jinn.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnchoredAssistantAutoPruneConfig {
    /// Whether the anchored-assistant auto-prune worker is active.
    /// Default: `true`.
    #[serde(default = "default_anchored_assistant_enabled")]
    pub enabled: bool,
    /// **Deprecated:** the radius value is now sourced from `AnchorShieldConfig.radius`.
    /// This field is kept for backward compatibility with existing `jinn.toml` files
    /// but its value is ignored at wiring time.
    ///
    /// Radius (in raw chat entries) within which an `Assistant` entry is
    /// protected from pruning, regardless of distance to any User entry.
    /// Distance strictly greater than this radius marks the entry as a
    /// prune candidate (subject to the `>80` token threshold).
    /// Minimum 1 (clamped at evaluation time).
    /// Default: `100`.
    #[serde(default = "default_anchored_assistant_radius")]
    pub radius: usize,
    /// Minimum number of entries from the end of history within which
    /// Assistant entries are protected from pruning even when both anchor
    /// distances exceed the radius. Counts every entry, regardless of
    /// in-context status. Set to 0 to disable protection.
    /// Default: `50`.
    #[serde(default = "default_anchored_assistant_min_age")]
    pub min_age: usize,
}

fn default_anchored_assistant_enabled() -> bool {
    DEFAULT_ANCHORED_ASSISTANT_ENABLED
}

fn default_anchored_assistant_radius() -> usize {
    DEFAULT_ANCHORED_ASSISTANT_RADIUS
}

fn default_anchored_assistant_min_age() -> usize {
    DEFAULT_ANCHORED_ASSISTANT_MIN_AGE
}

impl Default for AnchoredAssistantAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_ANCHORED_ASSISTANT_ENABLED,
            radius: DEFAULT_ANCHORED_ASSISTANT_RADIUS,
            min_age: DEFAULT_ANCHORED_ASSISTANT_MIN_AGE,
        }
    }
}
/// Default enabled state for anchor-shield auto-prune.
const DEFAULT_ANCHOR_SHIELD_ENABLED: bool = true;

/// Default radius (in raw history entries) for the anchor-shield worker.
const DEFAULT_ANCHOR_SHIELD_RADIUS: usize = 20;

/// Anchor-shield auto-prune strategy configuration.
///
/// Serialized as `[auto_prune.anchor_shield]` in `jinn.toml`.
///
/// The shield worker emits `ForcedInclude` for all in-context-by-default
/// entry types (`User`, `Assistant`, `ToolCall`, `ToolResult`) within
/// `radius` of any anchor entry. This prevents other workers from excluding
/// entries that carry conversation structure near user turns.
///
/// The `radius` value is also used by the `AnchoredAssistantAutoPruneWorker`
/// so the shield boundary and prune boundary always align.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnchorShieldConfig {
    /// Whether the anchor-shield worker is active.
    /// Default: `true`.
    #[serde(default = "default_anchor_shield_enabled")]
    pub enabled: bool,
    /// Radius (in raw chat entries) within which in-context entries
    /// are shielded from exclusion by other workers.
    /// This value is also used by the `AnchoredAssistantAutoPruneWorker`
    /// so the shield boundary and prune boundary always align.
    /// Minimum 1 (clamped at evaluation time).
    /// Default: `20`.
    #[serde(default = "default_anchor_shield_radius")]
    pub radius: usize,
}

fn default_anchor_shield_enabled() -> bool {
    DEFAULT_ANCHOR_SHIELD_ENABLED
}

fn default_anchor_shield_radius() -> usize {
    DEFAULT_ANCHOR_SHIELD_RADIUS
}

impl Default for AnchorShieldConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_ANCHOR_SHIELD_ENABLED,
            radius: DEFAULT_ANCHOR_SHIELD_RADIUS,
        }
    }
}
/// Default regex prune rule tool name.
const DEFAULT_REGEX_TOOL_NAME: &str = "bash";

/// Default regex prune rule keep_last.
const DEFAULT_REGEX_KEEP_LAST: usize = 1;

/// Default enabled state for regex auto-prune.
const DEFAULT_REGEX_ENABLED: bool = true;

/// Default minimum age for regex auto-prune.
const DEFAULT_REGEX_MIN_AGE: usize = 50;

/// A single regex-based auto-prune rule.
///
/// Serialized as `[[auto_prune.regex]]` in `jinn.toml`.
/// Each rule matches tool calls by name and content, keeping only the
/// most recent `keep_last` matching call+result pairs in context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Raw-distance protection floor: matching pairs whose `ToolCall` is within
    /// `min_age` slots of the end of history are never pruned by this rule.
    /// With `min_age = 0` no pair is protected (back-compat baseline).
    /// Default: 50.
    #[serde(default = "default_regex_min_age")]
    pub min_age: usize,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

fn default_regex_min_age() -> usize {
    DEFAULT_REGEX_MIN_AGE
}

impl Default for RegexAutoPruneConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_REGEX_ENABLED,
            rules: vec![
                RegexPruneRule {
                    pattern: "cargo test".to_owned(),
                    tool_name: DEFAULT_REGEX_TOOL_NAME.to_owned(),
                    keep_last: 2,
                    min_age: DEFAULT_REGEX_MIN_AGE,
                },
                RegexPruneRule {
                    pattern: "cargo check".to_owned(),
                    tool_name: DEFAULT_REGEX_TOOL_NAME.to_owned(),
                    keep_last: 1,
                    min_age: DEFAULT_REGEX_MIN_AGE,
                },
                RegexPruneRule {
                    pattern: "cargo clippy".to_owned(),
                    tool_name: DEFAULT_REGEX_TOOL_NAME.to_owned(),
                    keep_last: 1,
                    min_age: DEFAULT_REGEX_MIN_AGE,
                },
            ],
        }
    }
}

fn default_regex_enabled() -> bool {
    DEFAULT_REGEX_ENABLED
}

/// Auto-prune configuration.
///
/// Serialized as `[auto_prune]` in `jinn.toml`.
/// Groups all auto-prune strategy configurations.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoPruneConfig {
    /// Edit-read auto-prune strategy configuration.
    #[serde(default)]
    pub edit_read: EditReadAutoPruneConfig,
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
    /// Tool-age-window auto-prune strategy configuration.
    #[serde(default)]
    pub tool_age_window: ToolAgeWindowAutoPruneConfig,
    /// Trivial-assistant auto-prune strategy configuration.
    #[serde(default)]
    pub trivial_assistant: TrivialAssistantAutoPruneConfig,
    /// Anchored-assistant auto-prune strategy configuration.
    #[serde(default)]
    pub anchored_assistant: AnchoredAssistantAutoPruneConfig,
    /// Anchor-shield auto-prune strategy configuration.
    #[serde(default)]
    pub anchor_shield: AnchorShieldConfig,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebFetchConfig {
    /// The backend to use for web fetching. Default: `"headless-chrome"`.
    #[serde(default)]
    pub backend: WebFetchBackend,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            backend: WebFetchBackend::HeadlessChrome,
        }
    }
}

/// Default bash tool timeout in seconds (3 minutes).
const DEFAULT_BASH_DEFAULT_TIMEOUT_SECS: u64 = 180;

// serde default fns must return the field type (Option<u64>) even when always Some.
#[expect(
    clippy::unnecessary_wraps,
    reason = "trait contract requires Result return"
)]
fn default_bash_default_timeout_secs() -> Option<u64> {
    Some(DEFAULT_BASH_DEFAULT_TIMEOUT_SECS)
}

/// Bash tool configuration.
///
/// Serialized as `[bash]` in `jinn.toml`.
/// Controls the default execution timeout for the `bash` builtin tool.
/// The model can override per-call via the `timeout` JSON argument.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BashConfig {
    /// Default timeout in seconds for bash commands. Default: 180 (3 minutes).
    /// Set to `None` to disable the default timeout.
    #[serde(default = "default_bash_default_timeout_secs")]
    pub default_timeout_secs: Option<u64>,
}

impl Default for BashConfig {
    fn default() -> Self {
        Self {
            default_timeout_secs: Some(DEFAULT_BASH_DEFAULT_TIMEOUT_SECS),
        }
    }
}
/// OpenRouter web search server tool configuration.
///
/// Serialized as `[openrouter_web_search]` in `jinn.toml`.
/// Controls parameters sent to the `openrouter:web_search` server tool.
/// All fields are optional - when `None`, the parameter is omitted from
/// the request and OpenRouter uses its default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Bash tool configuration.
    #[serde(default)]
    pub bash: BashConfig,
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
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            bash: BashConfig::default(),
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
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            bash: BashConfig::default(),
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
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            bash: BashConfig::default(),
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
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            bash: BashConfig::default(),
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
            tool_entry_max_lines: None,
            min_collapse_count: None,
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
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            bash: BashConfig::default(),
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

    // --- Comment preservation tests for save_preferences ---

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
    fn save_preferences_preserves_session_lifecycle_block_and_comments() {
        // Given a jinn.toml with a session_lifecycle block.
        let original = "# my custom lifecycle\n[[session_lifecycle]]\nname = \"fossil-branch\"\ndescription = \"open a branch\"\n";
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, original).expect("write");

        // When loading and re-saving without changes.
        let prefs = load_preferences_from(&path).expect("load");
        save_preferences_to(&prefs, &path).expect("save");

        // Then the comment and entry are preserved.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(written.contains("# my custom lifecycle"));
        assert!(written.contains("name = \"fossil-branch\""));
    }

    #[rstest::rstest]
    fn save_preferences_deletes_session_lifecycle_block_on_struct_removal() {
        // Given a jinn.toml with two lifecycle blocks.
        let original = "# keep\n[[session_lifecycle]]\nname = \"alpha\"\n\n# delete\n[[session_lifecycle]]\nname = \"beta\"\n";
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, original).expect("write");

        // When loading and saving with only alpha kept.
        let mut prefs = load_preferences_from(&path).expect("load");
        prefs.session_lifecycles.retain(|l| l.name == "alpha");
        save_preferences_to(&prefs, &path).expect("save");

        // Then beta's block (and its comment) is removed.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(written.contains("# keep"));
        assert!(written.contains("name = \"alpha\""));
        assert!(!written.contains("beta"));
        assert!(!written.contains("# delete"));
    }

    #[rstest::rstest]
    fn save_preferences_appends_new_session_lifecycle_at_end() {
        // Given a jinn.toml with one lifecycle block.
        let original = "# existing\n[[session_lifecycle]]\nname = \"alpha\"\n";
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, original).expect("write");

        // When loading and adding a new lifecycle.
        let mut prefs = load_preferences_from(&path).expect("load");
        prefs.session_lifecycles.push(SessionLifecycle {
            name: "beta".to_owned(),
            ..Default::default()
        });
        save_preferences_to(&prefs, &path).expect("save");

        // Then beta appears after alpha.
        let written = std::fs::read_to_string(&path).expect("read");
        let alpha_pos = written.find("name = \"alpha\"").expect("alpha");
        let beta_pos = written.find("name = \"beta\"").expect("beta");
        assert!(alpha_pos < beta_pos);
        assert!(written.contains("# existing"));
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
        // Kills: replace load_preferences with Ok(Default::default()).
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
        // Kills: replace save_preferences with Ok(()).
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
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
            bash: BashConfig::default(),
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
    fn default_web_fetch_config_uses_headless_chrome_backend() {
        let config = WebFetchConfig::default();
        assert_eq!(config.backend, WebFetchBackend::HeadlessChrome);
    }

    #[rstest::rstest]
    fn default_preferences_has_headless_chrome_web_fetch() {
        let prefs = UserPreferences::default();
        assert_eq!(prefs.web_fetch.backend, WebFetchBackend::HeadlessChrome);
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
            "[minimap]
max_tokens = 5000
",
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
        assert!(config.edit_read.enabled);
        assert_eq!(config.edit_read.min_age, 50);
        assert_eq!(config.read_edit.threshold, 2);
        assert!(config.read_edit.enabled);
        assert!(config.todo.enabled);
        assert!(config.consecutive_reads.enabled);
        assert_eq!(config.consecutive_reads.keep_last, 5);
        assert_eq!(config.consecutive_reads.min_age, 80);
        assert!(config.tool_age_window.enabled);
        assert_eq!(config.read_edit.min_age, 50);
        assert_eq!(config.double_edit.min_age, 20);
        assert_eq!(config.tool_age_window.min_age, 150);
        assert!(config.trivial_assistant.enabled);
        assert_eq!(config.trivial_assistant.min_age, 100);
        assert_eq!(config.trivial_assistant.max_tokens, 80);
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
    fn load_parses_auto_prune_read_edit_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "[auto_prune.read_edit]
enabled = false
",
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
            "[auto_prune.consecutive_reads]
enabled = false
keep_last = 5
",
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(!prefs.auto_prune.consecutive_reads.enabled);
        assert_eq!(prefs.auto_prune.consecutive_reads.keep_last, 5);
    }

    #[rstest::rstest]
    fn load_parses_auto_prune_tool_age_window_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "[auto_prune.tool_age_window]
enabled = false
min_age = 50
",
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(!prefs.auto_prune.tool_age_window.enabled);
        assert_eq!(prefs.auto_prune.tool_age_window.min_age, 50);
    }

    #[rstest::rstest]
    fn load_parses_auto_prune_read_edit_min_age() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "[auto_prune.read_edit]
min_age = 25
",
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert_eq!(prefs.auto_prune.read_edit.min_age, 25);
    }

    #[rstest::rstest]
    fn load_parses_auto_prune_double_edit_min_age() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "[auto_prune.double_edit]
min_age = 15
",
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert_eq!(prefs.auto_prune.double_edit.min_age, 15);
    }

    #[rstest::rstest]
    fn config_without_min_age_uses_new_defaults() {
        // Given a TOML file with auto_prune sections that omit `min_age`, the new
        // defaults should kick in (50 for read_edit, 20 for double_edit, 100 for
        // tool_age_window).
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "[auto_prune.read_edit]
enabled = true

[auto_prune.double_edit]
enabled = true

[auto_prune.tool_age_window]
enabled = true
",
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert_eq!(prefs.auto_prune.read_edit.min_age, 50);
        assert_eq!(prefs.auto_prune.read_edit.threshold, 2);
        assert_eq!(prefs.auto_prune.double_edit.min_age, 20);
        assert_eq!(prefs.auto_prune.tool_age_window.min_age, 150);
    }

    #[rstest::rstest]
    fn save_then_load_round_trips_auto_prune_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            auto_prune: AutoPruneConfig {
                edit_read: EditReadAutoPruneConfig {
                    enabled: true,
                    min_age: 30,
                },
                read_edit: ReadEditAutoPruneConfig {
                    enabled: false,
                    min_age: 25,
                    threshold: 3,
                },
                regex: RegexAutoPruneConfig::default(),
                broken_edit: BrokenEditAutoPruneConfig {
                    enabled: false,
                    min_age: 3,
                },
                todo: TodoAutoPruneConfig {
                    enabled: false,
                    min_age: 0,
                },
                double_edit: DoubleEditAutoPruneConfig::default(),
                consecutive_reads: ConsecutiveReadsAutoPruneConfig::default(),
                tool_age_window: ToolAgeWindowAutoPruneConfig {
                    enabled: false,
                    min_age: 7,
                },
                trivial_assistant: TrivialAssistantAutoPruneConfig {
                    enabled: false,
                    min_age: 50,
                    max_tokens: 40,
                },
                anchored_assistant: AnchoredAssistantAutoPruneConfig {
                    enabled: false,
                    radius: 42,
                    min_age: 0,
                },
                anchor_shield: AnchorShieldConfig {
                    enabled: true,
                    radius: 20,
                },
            },
            ..UserPreferences::default()
        };

        save_preferences_to(&prefs, &path).expect("save");

        let reloaded = load_preferences_from(&path).expect("load");
        assert!(reloaded.auto_prune.edit_read.enabled);
        assert_eq!(reloaded.auto_prune.edit_read.min_age, 30);
        assert!(!reloaded.auto_prune.read_edit.enabled);
        assert_eq!(reloaded.auto_prune.read_edit.min_age, 25);
        assert_eq!(reloaded.auto_prune.read_edit.threshold, 3);
        assert!(!reloaded.auto_prune.broken_edit.enabled);
        assert_eq!(reloaded.auto_prune.broken_edit.min_age, 3);
        assert!(!reloaded.auto_prune.todo.enabled);
        assert!(!reloaded.auto_prune.tool_age_window.enabled);
        assert_eq!(reloaded.auto_prune.tool_age_window.min_age, 7);
        assert!(!reloaded.auto_prune.trivial_assistant.enabled);
        assert_eq!(reloaded.auto_prune.trivial_assistant.min_age, 50);
        assert_eq!(reloaded.auto_prune.trivial_assistant.max_tokens, 40);
        assert!(!reloaded.auto_prune.anchored_assistant.enabled);
        assert_eq!(reloaded.auto_prune.anchored_assistant.radius, 42);
        assert!(reloaded.auto_prune.anchor_shield.enabled);
        assert_eq!(reloaded.auto_prune.anchor_shield.radius, 20);
    }

    #[rstest::rstest]
    fn load_without_auto_prune_section_uses_defaults() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, "last_model = 'ollama/llama3'").expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(prefs.auto_prune.edit_read.enabled);
        assert!(prefs.auto_prune.read_edit.enabled);
        assert!(prefs.auto_prune.todo.enabled);
        assert!(prefs.auto_prune.tool_age_window.enabled);
        assert_eq!(prefs.auto_prune.tool_age_window.min_age, 150);
        assert!(prefs.auto_prune.trivial_assistant.enabled);
        assert_eq!(prefs.auto_prune.trivial_assistant.min_age, 100);
        assert_eq!(prefs.auto_prune.trivial_assistant.max_tokens, 80);
        assert!(prefs.auto_prune.anchored_assistant.enabled);
        assert_eq!(prefs.auto_prune.anchored_assistant.radius, 100);
        assert!(prefs.auto_prune.anchor_shield.enabled);
        assert_eq!(prefs.auto_prune.anchor_shield.radius, 20);
    }

    #[rstest::rstest]
    fn load_with_anchored_assistant_radius_uses_defaults_for_anchor_shield() {
        // Given a TOML file with only anchored_assistant radius (no anchor_shield section).
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "[auto_prune.anchored_assistant]
enabled = true
radius = 42
min_age = 5

[auto_prune.anchor_shield]
enabled = true
radius = 20
",
        )
        .expect("write");

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then anchored_assistant still reads its own radius.
        assert!(prefs.auto_prune.anchored_assistant.enabled);
        assert_eq!(prefs.auto_prune.anchored_assistant.radius, 42);
        assert_eq!(prefs.auto_prune.anchored_assistant.min_age, 5);

        // And anchor_shield uses its own config.
        assert!(prefs.auto_prune.anchor_shield.enabled);
        assert_eq!(prefs.auto_prune.anchor_shield.radius, 20);
    }

    #[rstest::rstest]
    fn load_parses_auto_prune_todo_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "[auto_prune.todo]
enabled = false
",
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(!prefs.auto_prune.todo.enabled);
        // edit_read should still have defaults
        assert!(prefs.auto_prune.edit_read.enabled);
        // read_edit should still have defaults
        assert!(prefs.auto_prune.read_edit.enabled);
    }

    #[rstest::rstest]
    fn default_min_age_is_50_for_new_workers() {
        // Given the three newly-min_age'd configs and the per-rule default.
        // Then their Default impls all produce min_age == 50.
        assert_eq!(AnchoredAssistantAutoPruneConfig::default().min_age, 50);
        // ConsecutiveReads min_age was raised to 80 in the defaults.
        // The other three remain 50.
        assert_eq!(ConsecutiveReadsAutoPruneConfig::default().min_age, 80);
        // RegexPruneRule has no Default impl (pattern is required), so verify
        // via the serde default function directly.
        assert_eq!(default_regex_min_age(), 50);
    }

    #[rstest::rstest]
    fn toml_roundtrip_with_renamed_fields() {
        // Given a TOML that uses the legacy aliases (`max_age_entries` and
        // `min_tail_entries`) instead of the new `min_age` field name.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "[auto_prune.trivial_assistant]
enabled = true
max_age_entries = 100
max_tokens = 80

[auto_prune.broken_edit]
enabled = true
min_tail_entries = 10
",
        )
        .expect("write");

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then the legacy keys are accepted via serde alias and populate
        // the new `min_age` field.
        assert_eq!(prefs.auto_prune.trivial_assistant.min_age, 100);
        assert_eq!(prefs.auto_prune.broken_edit.min_age, 10);
    }

    // --- RegexAutoPruneConfig tests ---

    #[rstest::rstest]
    fn default_regex_config_has_default_rules_and_enabled() {
        let config = RegexAutoPruneConfig::default();
        assert!(config.enabled);
        assert_eq!(config.rules.len(), 3);
        assert_eq!(config.rules[0].pattern, "cargo test");
        assert_eq!(config.rules[1].pattern, "cargo check");
        assert_eq!(config.rules[2].pattern, "cargo clippy");
    }

    #[rstest::rstest]
    fn regex_prune_rule_defaults_to_bash_tool_name() {
        let rule = RegexPruneRule {
            pattern: "cargo check".to_owned(),
            tool_name: default_regex_tool_name(),
            keep_last: default_regex_keep_last(),
            min_age: default_regex_min_age(),
        };
        assert_eq!(rule.tool_name, "bash");
        assert_eq!(rule.keep_last, 1);
        assert_eq!(rule.min_age, 50);
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
                            min_age: 50,
                        },
                        RegexPruneRule {
                            pattern: "cargo test".to_owned(),
                            tool_name: "bash".to_owned(),
                            keep_last: 2,
                            min_age: 50,
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
        assert_eq!(prefs.auto_prune.regex.rules.len(), 3);
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
                            min_age: 0,
                        },
                        RegexPruneRule {
                            pattern: "cargo check".to_owned(),
                            tool_name: "bash".to_owned(),
                            keep_last: 1,
                            min_age: 0,
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

    #[test]
    fn bash_config_default_is_180_secs() {
        // Given the default BashConfig.
        let cfg = BashConfig::default();

        // Then the default timeout is 180 seconds.
        assert_eq!(
            cfg.default_timeout_secs,
            Some(180),
            "BashConfig::default() must produce a 3-minute default timeout",
        );
    }

    #[test]
    fn bash_config_toml_roundtrip_explicit_value() {
        // Given a TOML fragment with an explicit override.
        let toml_str = "
            default_timeout_secs = 60
        ";

        // When parsed.
        let cfg: BashConfig = toml::from_str(toml_str).expect("parse");

        // Then the override round-trips.
        assert_eq!(cfg.default_timeout_secs, Some(60));

        // And serializing back preserves the value.
        let reserialized = toml::to_string(&cfg).expect("serialize");
        assert!(
            reserialized.contains("default_timeout_secs = 60"),
            "reserialized TOML must preserve the value; got: {reserialized}",
        );
    }

    #[test]
    fn bash_config_toml_empty_table_falls_back_to_default() {
        // Given an empty [bash] table in TOML.
        let toml_str = "";

        // When parsed.
        let cfg: BashConfig = toml::from_str(toml_str).expect("parse");

        // Then the serde default fn fires and produces 180.
        assert_eq!(
            cfg.default_timeout_secs,
            Some(180),
            "missing field should resolve via serde default fn",
        );
    }
}
