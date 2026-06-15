//! CWD input popup - type a directory path to change the session cwd.
//!
//! Provides a centered text-input popup (no TUI suspend) as a fast alternative to
//! the fzf/suspend flow bound to `<M-c>`/`<M-d>`. The user types a path; the footer
//! shows a live resolved path (green check) or error (red x). On confirm, the path
//! is resolved (`~` expand, relative-to-cwd, canonicalize), validated as an existing
//! dir, and the active session cwd is updated with context rescanned inline.

pub mod intent;
pub mod render;
pub mod resolve;
pub mod state;

use serde::{Deserialize, Serialize};

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
