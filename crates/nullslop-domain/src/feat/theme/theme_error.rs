//! Theme loading errors.

use wherror::Error;

/// Errors that can occur during theme loading and parsing.
#[derive(Debug, Error)]
pub enum ThemeError {
    /// Theme file not found.
    #[error("theme not found")]
    NotFound,
    /// TOML parsing error.
    #[error("theme parse error")]
    Parse,
    /// Invalid color value.
    #[error("invalid theme color")]
    InvalidColor,
    /// I/O error reading theme file.
    #[error("theme I/O error")]
    Io,
}
