//! Plugin error types.

use wherror::Error;

/// Errors that can occur during plugin lifecycle.
#[derive(Debug, Error)]
#[error(debug)]
pub enum PluginError {
    /// The plugin script could not be read from disk.
    LoadFailed,
    /// The plugin script has syntax errors.
    EvalFailed,
    /// The plugin's `init()` function threw an error.
    InitFailed,
    /// The plugin's `on_event()` function threw an error.
    OnEventFailed,
    /// The plugin is disabled and cannot execute.
    Disabled,
}
