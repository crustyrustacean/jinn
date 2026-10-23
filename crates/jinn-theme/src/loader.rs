//! Theme file discovery and loading.
//!
//! Scans the user themes directory for `*.toml` files, parses them, and
//! resolves missing fields from the default theme.

use marked_path::{Absolute, MarkedPath, MarkedPathAccess, MarkedPathBuf};

use error_stack::{Report, ResultExt as _};
use wherror::Error;

use crate::default_theme;
use crate::theme::{Theme, ThemeFile};

/// Errors that can occur during theme loading and parsing.
#[derive(Debug, Error, PartialEq, Eq)]
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

/// Discovers all theme TOML files in the given themes directory.
///
/// Returns a list of `(name, path)` pairs, where `name` is the filename
/// without the `.toml` extension. Returns an empty list if the directory
/// doesn't exist.
///
/// # Errors
///
/// Returns [`ThemeError::Io`] if the directory cannot be read.
pub fn discover_themes(
    themes_dir: &MarkedPath<Absolute>,
) -> Result<Vec<(String, MarkedPathBuf<Absolute>)>, Report<ThemeError>> {
    if !themes_dir.exists() {
        return Ok(Vec::new());
    }

    let mut themes = Vec::new();
    let entries = std::fs::read_dir(themes_dir)
        .change_context(ThemeError::Io)
        .attach("failed to read themes directory")?;

    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Ok(typed) = MarkedPathBuf::<Absolute>::new(entry.path()) else {
            continue;
        };
        if typed.extension().is_some_and(|ext| ext == "toml")
            && let Some(name) = typed.file_stem().and_then(|s| s.to_str())
        {
            themes.push((name.to_owned(), typed));
        }
    }

    themes.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(themes)
}

/// Loads a theme by name from the themes directory.
///
/// Looks for `<dir>/<name>.toml`. If not found, returns
/// [`ThemeError::NotFound`].
///
/// # Errors
///
/// - [`ThemeError::NotFound`] - no TOML file with that name exists in the dir.
/// - [`ThemeError::Parse`] - TOML is malformed or contains invalid colors.
/// - [`ThemeError::Io`] - file cannot be read.
pub fn load_theme(name: &str, dir: &MarkedPath<Absolute>) -> Result<Theme, Report<ThemeError>> {
    let path = dir
        .join(name)
        .map(|joined| joined.with_added_extension("toml"))
        .change_context(ThemeError::Io)
        .attach("failed to build theme path")?;
    if !path.exists() {
        return Err(Report::new(ThemeError::NotFound)
            .attach(format!("theme file not found: {}", path.to_string_lossy())));
    }
    load_theme_from_file(&path.as_marked_path())
}

/// Loads a theme from a specific file path.
///
/// Missing fields in the TOML are filled from the default theme.
///
/// # Errors
///
/// - [`ThemeError::Parse`] - TOML is malformed or contains invalid colors.
/// - [`ThemeError::Io`] - file cannot be read.
pub fn load_theme_from_file(path: &MarkedPath<Absolute>) -> Result<Theme, Report<ThemeError>> {
    let content = std::fs::read_to_string(path)
        .change_context(ThemeError::Io)
        .attach(format!(
            "failed to read theme file: {}",
            path.to_string_lossy()
        ))?;

    let file: ThemeFile = toml::from_str(&content)
        .change_context(ThemeError::Parse)
        .attach(format!(
            "failed to parse theme file: {}",
            path.to_string_lossy()
        ))?;

    Ok(file.resolve())
}

/// Resolves a theme name to a `Theme` from the themes directory.
///
/// - If `name` is `None` or `"default"`: tries loading `default.toml` from
///   the dir. If not found, falls back to the embedded default theme.
/// - Otherwise, loads the named theme from the dir.
///
/// # Errors
///
/// Propagates errors from [`load_theme`] for non-default theme names.
/// For the default theme, only returns errors for parse/IO failures
/// (not-found silently falls back to embedded).
pub fn resolve_theme(
    name: Option<&str>,
    themes_dir: &MarkedPath<Absolute>,
) -> Result<Theme, Report<ThemeError>> {
    match name {
        None | Some("default") => {
            // A default.toml in the dir allows user customization.
            match load_theme("default", themes_dir) {
                Ok(theme) => Ok(theme),
                Err(err) if err.downcast_ref::<ThemeError>() == Some(&ThemeError::NotFound) => {
                    // File not found - use embedded default.
                    Ok(default_theme())
                }
                Err(err) => Err(err),
            }
        }
        Some(name) => load_theme(name, themes_dir),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;

    #[rstest::rstest]
    fn discover_themes_returns_empty_for_missing_dir() {
        // Given a themes directory that doesn't exist.
        let root = jinn_testutil::tempdir();
        let dir = root
            .path
            .join_all(["nonexistent", "path", "themes"])
            .expect("name joins keep the base absolute");

        // When discovering themes.
        let result = discover_themes(&dir.as_marked_path());

        // Then it returns Ok with an empty list.
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            [] as [(
                std::string::String,
                marked_path::MarkedPathBuf<marked_path::Absolute>
            ); 0]
        );
    }

    #[rstest::rstest]
    fn load_theme_returns_not_found_for_missing_file() {
        // Given a themes directory with no such theme file.
        let dir = jinn_testutil::tempdir();

        // When loading a theme name that doesn't exist.
        let result = load_theme("nonexistent_theme_xyz", &dir.path.as_marked_path());

        // Then it returns NotFound error.
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .downcast_ref::<ThemeError>()
                .is_some_and(|e| matches!(e, ThemeError::NotFound)),
            "expected NotFound error"
        );
    }

    #[rstest::rstest]
    fn load_theme_from_file_parses_valid_toml() {
        // Given a valid theme TOML file.
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("test.toml")
            .expect("temp root joins a bare name");
        std::fs::write(&path, "focus_accent = \"red\"\nprimary_text = \"#FFFFFF\"").expect("write");

        // When loading.
        let theme = load_theme_from_file(&path.as_marked_path()).expect("load");

        // Then the specified fields are set.
        assert_eq!(theme.focus_accent, ratatui::style::Color::Red);
        assert_eq!(
            theme.primary_text,
            ratatui::style::Color::Rgb(255, 255, 255)
        );
        // And unspecified fields fall back to default.
        assert_eq!(theme.muted_text, default_theme().muted_text);
    }

    #[rstest::rstest]
    fn load_theme_from_file_rejects_invalid_toml() {
        // Given an invalid TOML file.
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("bad.toml")
            .expect("temp root joins a bare name");
        std::fs::write(&path, "this is not valid toml {{{{").expect("write");

        // When loading.
        let result = load_theme_from_file(&path.as_marked_path());

        // Then it returns a Parse error.
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .downcast_ref::<ThemeError>()
                .is_some_and(|e| matches!(e, ThemeError::Parse)),
            "expected Parse error"
        );
    }

    #[rstest::rstest]
    fn load_theme_from_file_rejects_invalid_color() {
        // Given a TOML file with an invalid color value.
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("bad-color.toml")
            .expect("temp root joins a bare name");
        std::fs::write(&path, "focus_accent = \"not_a_color\"").expect("write");

        // When loading.
        let result = load_theme_from_file(&path.as_marked_path());

        // Then it returns a Parse error.
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .downcast_ref::<ThemeError>()
                .is_some_and(|e| matches!(e, ThemeError::Parse)),
            "expected Parse error"
        );
    }

    #[rstest::rstest]
    fn resolve_theme_none_returns_default_when_no_file() {
        // Given an empty themes directory.
        let dir = jinn_testutil::tempdir();

        // When resolving with None.
        let theme = resolve_theme(None, &dir.path.as_marked_path()).expect("resolve");

        // Then it returns the embedded default theme.
        assert_eq!(theme.focus_accent, default_theme().focus_accent);
    }

    #[rstest::rstest]
    fn resolve_theme_default_string_returns_default_when_no_file() {
        // Given an empty themes directory.
        let dir = jinn_testutil::tempdir();

        // When resolving with "default".
        let theme = resolve_theme(Some("default"), &dir.path.as_marked_path()).expect("resolve");

        // Then it returns the embedded default theme.
        assert_eq!(theme.focus_accent, default_theme().focus_accent);
    }

    #[rstest::rstest]
    fn resolve_theme_default_loads_from_filesystem() {
        // Given a themes directory with a custom default.toml.
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("default.toml")
            .expect("temp root joins a bare name");
        std::fs::write(&path, "focus_accent = \"red\"").expect("write");

        // When resolving with "default".
        let theme = resolve_theme(Some("default"), &dir.path.as_marked_path()).expect("resolve");

        // Then it loads from the filesystem (focus_accent overridden).
        assert_eq!(theme.focus_accent, ratatui::style::Color::Red);
        // And other fields fall back to embedded default.
        assert_eq!(theme.muted_text, default_theme().muted_text);
    }

    #[rstest::rstest]
    fn discover_themes_finds_toml_files() {
        // Given a temp directory with theme files.
        let dir = jinn_testutil::tempdir();
        std::fs::write(
            dir.path
                .join("ocean.toml")
                .expect("temp root joins a bare name"),
            "focus_accent = \"blue\"",
        )
        .expect("write");
        std::fs::write(
            dir.path
                .join("forest.toml")
                .expect("temp root joins a bare name"),
            "focus_accent = \"green\"",
        )
        .expect("write");
        std::fs::write(
            dir.path
                .join("readme.txt")
                .expect("temp root joins a bare name"),
            "not a theme",
        )
        .expect("write");

        // When discovering themes.
        let themes = discover_themes(&dir.path.as_marked_path()).expect("discover");

        // Then only .toml files are found, sorted by name.
        assert_eq!(themes.len(), 2);
        assert_eq!(themes[0].0, "forest");
        assert_eq!(themes[1].0, "ocean");
    }
}
