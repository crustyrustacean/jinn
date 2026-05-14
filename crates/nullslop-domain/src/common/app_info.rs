//! Application identity constants.
//!
//! Re-exports [`APP_NAME`] from `nullslop_plugin` and adds domain-level
//! derived constants (e.g., preferences file name).

pub use nullslop_plugin::app_info::APP_NAME;

/// The user preferences file name, derived from [`APP_NAME`].
pub const PREFS_FILE_NAME: &str = "nullslop.toml";
