//! Project-path → repo-basename derivation.
//!
//! Lifecycle scripts like `fossil branch w/quickfix` take a `<repo>` parameter
//! that names the Fossil checkout to branch. By convention the repo name is the
//! last path segment of the project directory (e.g. `/mnt/zed/repos/jinn` →
//! `jinn`). This module derives it without string slicing.

use std::path::Path;

/// Derive the repo basename from a project path.
///
/// Returns the final path component (`Path::file_name`), falling back to the
/// input verbatim if the path has no final component (e.g. `/` or `..`).
///
/// # Examples
///
/// ```
/// # use jinn_domain::feat::discord::repo_basename::repo_basename;
/// assert_eq!(repo_basename("/mnt/zed/repos/jinn"), "jinn");
/// assert_eq!(repo_basename("/mnt/zed/repos/my-app"), "my-app");
/// ```
#[must_use]
pub fn repo_basename(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|os| os.to_str())
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::repo_basename;

    #[test]
    fn unix_repo_path_yields_last_segment() {
        // Given a typical repo path.
        // When deriving the basename.
        // Then the last segment is returned.
        assert_eq!(repo_basename("/mnt/zed/repos/jinn"), "jinn");
    }

    #[test]
    fn hyphenated_segment_is_preserved() {
        // Given a repo path with a hyphen in the last segment.
        // When deriving the basename.
        // Then the hyphenated name is returned intact.
        assert_eq!(repo_basename("/mnt/zed/repos/my-app"), "my-app");
    }
}
