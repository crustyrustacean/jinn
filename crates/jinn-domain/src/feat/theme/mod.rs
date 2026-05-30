//! Theme re-exports from the `jinn-theme` crate.
//!
//! All theme types, loading, and contrast logic have been extracted into
//! the `jinn-theme` crate. This module re-exports everything so that
//! existing consumers using `crate::feat::theme::*` continue to work.

// Re-export sub-modules for direct path access (e.g., `crate::feat::theme::contrast::darken`).
pub use jinn_theme::contrast;

// Re-export individual types and functions.
pub use jinn_theme::{
    Theme, ThemeColor, ThemeEntry, ThemeError, default_theme, discover_themes, load_theme,
    load_theme_from_dir, resolve_theme, resolve_theme_from_dir,
};
