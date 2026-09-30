//! The `[[tools.bash_command_policy]]` section — the global, non-project
//! command rules the bash tool consults.
//!
//! Advisory-strength by design: rules exist to stop well-trained habits
//! (like per-package test invocations in a whole-workspace repo), not to
//! resist a determined actor.
//!
//! This module holds the section's declaration and its value shape only.
//! Resolution — merging the global rules with a project's own — and the
//! compiled matcher that runs them live in the `jinn-tools` slice.

/// The `jinn.toml` key the global (non-project-specific) rules live at.
pub const GLOBAL_COMMAND_POLICY_KEY: &str = "tools.bash_command_policy";

/// A rule pairs a user-authored regex with the corrective message returned
/// when the regex matches a command. Rules are advisory-strength by design:
/// they exist to stop well-trained habits, not to resist a determined actor.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommandPolicyRule {
    /// Regex matched against the full command string.
    pub pattern: String,
    /// Message returned in the failed tool result when [`Self::pattern`] matches.
    pub message: String,
}

impl jinn_config::ConfigList for CommandPolicyRule {
    const KEY: &'static str = GLOBAL_COMMAND_POLICY_KEY;
    const ENTRY_KEY: &'static str = "pattern";
    const ENTRY_FIELDS: &'static [&'static str] = &["pattern", "message"];
}
