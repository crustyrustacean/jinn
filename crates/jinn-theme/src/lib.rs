//! Dynamic theme support for the TUI.
//!
//! Provides a [`Theme`] struct with semantic color fields, TOML-based theme file
//! loading, and a default theme that matches the current hardcoded colors. Themes
//! are discovered from `~/.config/jinn/themes/*.toml`.
//!
//! # Color formats in TOML
//!
//! Theme TOML files support four color formats:
//!
//! ```toml
//! # ANSI name (ratatui Color enum name, case-insensitive)
//! focus_accent = "yellow"
//!
//! # Hex string
//! primary_text = "#ffffff"
//!
//! # RGB array
//! gutter_bg = [25, 27, 30]
//!
//! # ANSI code: A0-A15 for 4-bit, A0-A255 for extended 256-color
//! focus_accent = "A80"
//! ```
//!
//! Missing fields fall back to the default theme values.

pub mod color;
#[cfg(test)]
mod color_tests;
pub mod contrast;
pub mod default_theme;
pub mod loader;
pub mod theme;
pub mod theme_entry;
pub use color::ThemeColor;
pub use default_theme::default_theme;
pub use loader::ThemeError;
pub use loader::{
    discover_themes, load_theme, load_theme_from_dir, resolve_theme, resolve_theme_from_dir,
};
pub use theme::Theme;
pub use theme_entry::ThemeEntry;
