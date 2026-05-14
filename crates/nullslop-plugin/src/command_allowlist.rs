//! Command allowlist — static list of commands plugins may send.

/// Returns `true` if the command name is on the allowlist.
#[must_use]
pub fn is_allowed(name: &str) -> bool {
    matches!(name, "cancel_stream")
}
