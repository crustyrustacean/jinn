//! `jinn.toml` default-config bootstrap.
//!
//! `jinn.toml` has no aggregate schema struct. Each section is declared
//! by its owning slice through a `Configurable` impl and read through
//! the configuration layer. What lives here is the comment-rich template
//! a fresh install is seeded from, and the helpers that write it.
//!
//! The file lives at
//! `~/.config/jinn/jinn.toml` and is auto-created on first run from
//! [`DEFAULT_CONFIG`] (a comment-rich template embedded at compile time).

use std::path::{Path, PathBuf};

use error_stack::{Report, ResultExt as _};
use jinn_common::app_info::{APP_NAME, PREFS_FILE_NAME};
use serde::{Deserialize, Serialize};
use wherror::Error;

// ── Embedded config schemas ─────────────────────────────────────────────
// The section types live in `crate::schemas` (co-located by feature
// domain) and are re-exported so a consumer has one import home for
// `jinn.toml` shapes.
pub use crate::schemas::{
    AutoPruneConfig, CompactionConfig, CwdSelectorConfig, MinimapConfig, ProjectConfig,
    RequestRetryConfig, SessionLifecycle, StallWatchdogConfig, ToolCallWatchdogConfig,
};

/// Canonical default `jinn.toml` embedded at compile time.
///
/// Used both to auto-create the file on first run and to back the
/// `jinn config init` subcommand. The template is independent of the
/// struct's default values; `template_validation_tests` guarantees it
/// parses, documents every config key, and activates into a valid
/// config.
pub const DEFAULT_CONFIG: &str = include_str!("default_jinn.toml");
/// Default execution timeout (seconds) for all tool calls.
///
/// The model can override per-call via the reserved `max_duration_secs` argument
/// (supported by `bash`); a value of `0` disables the timeout for that call.
pub(crate) const DEFAULT_TOOL_DEFAULT_TIMEOUT_SECS: u64 = 300;

/// The built-in tool timeout a fresh install starts with.
pub fn default_tool_default_timeout_secs() -> u64 {
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

/// Writes the canonical default preferences template to `path`.
///
/// Creates parent directories as needed.
///
/// Writes the template as bytes. It deliberately does not round-trip
/// through [`UserPreferences`]: the template is documentation, and
/// serializing a struct would strip every comment it ships with.
///
/// # Errors
///
/// Returns [`UserPreferencesError::Io`] if directory creation or file writing fails.
pub fn create_default_preferences_to<P>(path: P) -> Result<(), Report<UserPreferencesError>>
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
