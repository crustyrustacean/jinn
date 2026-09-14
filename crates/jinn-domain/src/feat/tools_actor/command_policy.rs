//! Project command policy - resolution and matching for the bash tool.
//!
//! Resolves the blocked-command rules of the configured project containing a
//! session's cwd (`~`-expanded lexical longest-prefix match) and compiles them
//! into a matcher consulted by the bash tool before any child process spawns.
//!
//! Advisory-strength by design: rules exist to stop well-trained habits
//! (like `cargo test -p` in a whole-workspace repo), not to resist a
//! determined actor.

use std::path::{Path, PathBuf};

use regex::Regex;

use crate::feat::project::{CommandPolicyRule, ProjectConfig};

/// Compiled blocked-command rules for one project. Empty matches nothing.
#[derive(Debug, Clone, Default)]
pub struct CompiledCommandPolicy {
    /// Compiled regexes paired with their messages, in config order.
    rules: Vec<(Regex, String)>,
}

impl CompiledCommandPolicy {
    /// Compiles user-authored rules. An invalid regex is skipped with a
    /// `tracing::warn!` naming the pattern — one bad rule is inert, never
    /// fatal for the project or the tool call.
    #[must_use]
    pub fn compile(rules: &[CommandPolicyRule]) -> Self {
        let compiled: Vec<(Regex, String)> = rules
            .iter()
            .filter_map(|rule| match Regex::new(&rule.pattern) {
                Ok(regex) => Some((regex, rule.message.clone())),
                Err(err) => {
                    tracing::warn!(
                        pattern = %rule.pattern,
                        %err,
                        "command_policy: skipping rule with invalid regex"
                    );
                    None
                }
            })
            .collect();
        Self { rules: compiled }
    }

    /// Returns `(pattern_as_written, message)` for the first rule matching
    /// `command`. Config order is precedence: first match wins.
    #[must_use]
    pub fn matched_message(&self, command: &str) -> Option<(&str, &str)> {
        self.rules
            .iter()
            .find(|(regex, _)| regex.is_match(command))
            .map(|(regex, message)| (regex.as_str(), message.as_str()))
    }

    /// True when no rules are compiled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// Returns the command-policy rules of the configured project containing
/// `cwd`, or an empty vec.
///
/// Matching is a lexical, component-wise prefix between each `~`-expanded
/// project path and `cwd`; the longest matching project path wins (nesting).
#[must_use]
pub fn resolve_project_rules(
    projects: &[ProjectConfig],
    cwd: &Path,
    home: &Path,
) -> Vec<CommandPolicyRule> {
    matching_project(projects, cwd, home)
        .map_or_else(Vec::new, |project| project.command_policy.clone())
}

/// Returns the configured project with the longest `~`-expanded path that is
/// a lexical prefix of (or equal to) `cwd`.
fn matching_project<'a>(
    projects: &'a [ProjectConfig],
    cwd: &Path,
    home: &Path,
) -> Option<&'a ProjectConfig> {
    projects
        .iter()
        .filter_map(|project| {
            let expanded = expand_tilde(&project.path, home);
            cwd.starts_with(&expanded)
                .then_some((expanded.as_os_str().len(), project))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, project)| project)
}

/// Expands a leading `~` (and `~/`) in a configured project path against `home`.
fn expand_tilde(path: &Path, home: &Path) -> PathBuf {
    let Some(first) = path.components().next() else {
        return path.to_path_buf();
    };
    match first {
        std::path::Component::Normal(marker) if marker == "~" => {
            let suffix: PathBuf = path.components().skip(1).collect();
            if suffix.as_os_str().is_empty() {
                home.to_path_buf()
            } else {
                home.join(suffix)
            }
        }
        _ => path.to_path_buf(),
    }
}
