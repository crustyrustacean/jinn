//! Path display transforms shared across slices.
//!
//! Several slices render a path into a single-line label, and all of them want
//! the same treatment: collapse the home directory to `~`. The transform is pure
//! and has no slice-specific meaning, so it lives here rather than in whichever
//! slice happened to need it first.

use std::path::Path;

/// Shorten a path for display: replace the home directory prefix with `~`.
///
/// Paths under `$HOME` collapse to `~/…` (or just `~` when the path *is* the home
/// directory); any other path is returned unchanged as a display string. Falls
/// back to the raw path when `dirs::home_dir()` cannot be determined.
///
/// This is a pure display transform. The cwd slice's `resolve_cwd_input` expands
/// `~` back when resolving user input, so shortened paths round-trip.
#[must_use]
pub fn shorten_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(relative) = path.strip_prefix(&home)
    {
        let display = relative.display().to_string();
        if display.is_empty() {
            return "~".to_owned();
        }
        return format!("~/{display}");
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::shorten_path;
    use std::path::Path;

    #[rstest::rstest]
    fn path_outside_home_is_returned_unchanged() {
        // Given a path outside the home directory.
        let path = Path::new("/opt/jinn/config.toml");

        // When shortening it.
        let display = shorten_path(path);

        // Then the full path is shown.
        assert_eq!(display, "/opt/jinn/config.toml");
    }

    #[rstest::rstest]
    fn home_relative_path_collapses_to_tilde() {
        // Given a home directory.
        let Some(home) = dirs::home_dir() else {
            return;
        };

        // And a path beneath it.
        let path = home.join("projects/jinn");

        // When shortening it.
        let display = shorten_path(&path);

        // Then the home prefix is replaced with a tilde.
        assert_eq!(display, "~/projects/jinn");
    }

    #[rstest::rstest]
    fn home_itself_shortens_to_bare_tilde() {
        // Given the home directory itself.
        let Some(home) = dirs::home_dir() else {
            return;
        };

        // When shortening it.
        let display = shorten_path(&home);

        // Then only a tilde remains.
        assert_eq!(display, "~");
    }
}
