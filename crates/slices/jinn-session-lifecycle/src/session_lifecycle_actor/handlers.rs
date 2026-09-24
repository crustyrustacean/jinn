//! Handler modules and shared lifecycle presentation helpers.

mod close;
mod cwd;
mod setup;
mod teardown;

use std::path::Path;

use jinn_core_types::ChatEntry;
use strip_ansi_escapes::strip_str;

/// Removes ANSI escape sequences from command output.
#[must_use]
pub(super) fn strip_ansi(value: &str) -> String {
    strip_str(value)
}

/// Builds the informational entry shown when setup returns no path.
#[must_use]
pub(super) fn no_output_info(default_cwd: &Path) -> ChatEntry {
    ChatEntry::system(format!(
        "No path returned by setup command. Using {} as cwd",
        default_cwd.display()
    ))
}

/// Builds the successful setup entry.
#[must_use]
pub(super) fn setup_complete_msg(cwd: &Path) -> ChatEntry {
    ChatEntry::system(format!(
        "✅ Setup complete - Using {} as cwd",
        cwd.display()
    ))
}

/// Builds the entry shown while teardown is running.
#[must_use]
pub(super) fn teardown_running_msg() -> ChatEntry {
    ChatEntry::system("⚙️ Running teardown script...")
}

/// Builds the successful teardown-only entry.
#[must_use]
pub(super) fn teardown_success_msg() -> ChatEntry {
    ChatEntry::system("✅ Teardown completed successfully.")
}
