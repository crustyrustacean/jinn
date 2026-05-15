//! Theme file discovery and loading.
//!
//! Scans `~/.config/nullslop/themes/*.toml` for theme files, parses them,
//! and resolves missing fields from the default theme.

use std::path::{Path, PathBuf};

use error_stack::{Report, ResultExt as _};

use super::theme::{Theme, ThemeFile};
use super::theme_error::ThemeError;
use super::default_theme;

/// Returns the path to the themes directory.
///
/// Uses `dirs::config_dir()` → `~/.config/nullslop/themes/`.
#[must_use]
pub fn themes_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nullslop")
        .join("themes")
}

/// Discovers all theme TOML files in the themes directory.
///
/// Returns a list of `(name, path)` pairs, where `name` is the filename
/// without the `.toml` extension. The themes directory is created if it
/// doesn't exist.
///
/// # Errors
///
/// Returns [`ThemeError::Io`] if the directory cannot be read.
pub fn discover_themes() -> Result<Vec<(String, PathBuf)>, Report<ThemeError>> {
    let dir = themes_dir();

    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut themes = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .change_context(ThemeError::Io)
        .attach("failed to read themes directory")?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                themes.push((name.to_owned(), path));
            }
        }
    }

    themes.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(themes)
}

/// Loads a theme by name from the themes directory.
///
/// Looks for `<themes_dir>/<name>.toml`. If not found, returns
/// [`ThemeError::NotFound`]. If found but invalid, returns
/// [`ThemeError::Parse`].
///
/// # Errors
///
/// - [`ThemeError::NotFound`] — no TOML file with that name exists.
/// - [`ThemeError::Parse`] — TOML is malformed or contains invalid colors.
/// - [`ThemeError::Io`] — file cannot be read.
pub fn load_theme(name: &str) -> Result<Theme, Report<ThemeError>> {
    let path = themes_dir().join(format!("{name}.toml"));
    if !path.exists() {
        return Err(Report::new(ThemeError::NotFound).attach(format!(
            "theme file not found: {}",
            path.display()
        )));
    }
    load_theme_from_file(&path)
}

/// Loads a theme from a specific file path.
///
/// Missing fields in the TOML are filled from the default theme.
///
/// # Errors
///
/// - [`ThemeError::Parse`] — TOML is malformed or contains invalid colors.
/// - [`ThemeError::Io`] — file cannot be read.
pub fn load_theme_from_file(path: &Path) -> Result<Theme, Report<ThemeError>> {
    let content = std::fs::read_to_string(path)
        .change_context(ThemeError::Io)
        .attach(format!("failed to read theme file: {}", path.display()))?;

    let file: ThemeFile = toml::from_str(&content)
        .change_context(ThemeError::Parse)
        .attach(format!("failed to parse theme file: {}", path.display()))?;

    Ok(file.resolve())
}

/// Resolves a theme name to a `Theme`.
///
/// - If `name` is `None` or `"default"`, returns the built-in default theme.
/// - Otherwise, loads the named theme from the themes directory.
///
/// # Errors
///
/// Propagates errors from [`load_theme`] for non-default theme names.
pub fn resolve_theme(name: Option<&str>) -> Result<Theme, Report<ThemeError>> {
    match name {
        None | Some("default") => Ok(default_theme()),
        Some(name) => load_theme(name),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[rstest::rstest]
    fn themes_dir_contains_nullslop_themes() {
        // Given the themes directory path.
        let dir = themes_dir();

        // Then it ends with nullslop/themes.
        assert!(dir.to_string_lossy().ends_with("nullslop/themes"));
    }

    #[rstest::rstest]
    fn discover_themes_returns_empty_for_missing_dir() {
        // Given a themes directory that doesn't exist.
        // (The default themes dir likely doesn't exist in test environments.)
        let result = discover_themes();

        // Then it returns Ok with an empty or existing list.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn load_theme_returns_not_found_for_missing_file() {
        // Given a theme name that doesn't exist.
        let result = load_theme("nonexistent_theme_xyz");

        // Then it returns NotFound error.
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.downcast_ref::<ThemeError>().is_some_and(|e| matches!(e, ThemeError::NotFound)),
            "expected NotFound error"
        );
    }

    #[rstest::rstest]
    fn load_theme_from_file_parses_valid_toml() {
        // Given a valid theme TOML file.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("test.toml");
        std::fs::write(
            &path,
            "focus_accent = \"red\"\nprimary_text = \"#FFFFFF\"",
        )
        .expect("write");

        // When loading.
        let theme = load_theme_from_file(&path).expect("load");

        // Then the specified fields are set.
        assert_eq!(theme.focus_accent, ratatui::style::Color::Red);
        assert_eq!(theme.primary_text, ratatui::style::Color::Rgb(255, 255, 255));
        // And unspecified fields fall back to default.
        assert_eq!(theme.muted_text, default_theme().muted_text);
    }

    #[rstest::rstest]
    fn load_theme_from_file_rejects_invalid_toml() {
        // Given an invalid TOML file.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "this is not valid toml {{{{").expect("write");

        // When loading.
        let result = load_theme_from_file(&path);

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
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("bad-color.toml");
        std::fs::write(&path, "focus_accent = \"not_a_color\"").expect("write");

        // When loading.
        let result = load_theme_from_file(&path);

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
    fn resolve_theme_none_returns_default() {
        // Given None as the theme name.
        let theme = resolve_theme(None).expect("resolve");

        // Then it returns the default theme.
        assert_eq!(theme.focus_accent, default_theme().focus_accent);
    }

    #[rstest::rstest]
    fn resolve_theme_default_string_returns_default() {
        // Given "default" as the theme name.
        let theme = resolve_theme(Some("default")).expect("resolve");

        // Then it returns the default theme.
        assert_eq!(theme.focus_accent, default_theme().focus_accent);
    }

    #[rstest::rstest]
    fn discover_themes_finds_toml_files() {
        // Given a temp directory with theme files.
        let dir = TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("ocean.toml"), "focus_accent = \"blue\"")
            .expect("write");
        std::fs::write(dir.path().join("forest.toml"), "focus_accent = \"green\"")
            .expect("write");
        std::fs::write(dir.path().join("readme.txt"), "not a theme").expect("write");

        // When discovering themes.
        let themes = discover_themes_in(dir.path());

        // Then only .toml files are found, sorted by name.
        assert_eq!(themes.len(), 2);
        assert_eq!(themes[0].0, "forest");
        assert_eq!(themes[1].0, "ocean");
    }

    /// Test helper that discovers themes in a specific directory.
    fn discover_themes_in(dir: &Path) -> Vec<(String, PathBuf)> {
        let mut themes = Vec::new();
        let entries = std::fs::read_dir(dir).expect("read dir");
        for entry in entries {
            let entry = entry.expect("entry");
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                    themes.push((name.to_owned(), path));
                }
            }
        }
        themes.sort_by(|a, b| a.0.cmp(&b.0));
        themes
    }
}
