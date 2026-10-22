//! Typed path construction for tests.
//!
//! A test that builds a path by string concatenation can produce a path that
//! names one file on Linux and a different one on Windows: a forward slash
//! where a backslash belongs, a drive prefix that resolves against the wrong
//! working directory, a literal that only exists on the host that wrote it.
//!
//! This module hands out paths whose *shape* is carried by the type, not by
//! convention. A [`TestRoot`] owns an absolute root; the only way to build a
//! path inside it is [`TestRoot::join_rel`], which takes a
//! [`MarkedPath<Relative>`] — a value that is guaranteed relative and
//! guaranteed free of drive prefixes. The result is an absolute path, and the
//! two cannot be confused.
//!
//! ```
//! use jinn_testutil::paths::TestRoot;
//! use marked_path::{MarkedPathAccess, MarkedPathBuf, Relative};
//!
//! let root = TestRoot::new();
//! let name = MarkedPathBuf::<Relative>::new("notes.txt").expect("valid relative");
//! let path = root.join_rel(&name.as_marked_path());
//! // Query methods come from the `MarkedPathAccess` trait.
//! assert!(path.as_path().is_absolute());
//! ```

use std::path::PathBuf;

use marked_path::MarkedPathAccess;
use marked_path::{Absolute, MarkedPath, MarkedPathBuf, PathError, Relative};

pub use jinn_common::fs_paths::{canonicalize_display, strip_verbatim_prefix};

use crate::temp::tempdir;

/// A temporary directory that owns an absolute, typed root path.
///
/// The directory is created with the jinn test prefix (see
/// [`TEST_PREFIX`]), so residue from a crashed run is greppable and
/// hand-removable without touching another process's files.
pub struct TestRoot {
    dir: crate::temp::JinnTempDir,
    root: MarkedPathBuf<Absolute>,
}

impl Default for TestRoot {
    /// Creates a fresh temporary directory and captures its absolute root.
    ///
    /// # Panics
    ///
    /// Panics if the temporary directory cannot be created — a failed fixture
    /// aborts the test loudly, which is the desired mode.
    fn default() -> Self {
        Self::new()
    }
}

impl TestRoot {
    /// Creates a fresh temporary directory and captures its absolute root.
    ///
    /// # Panics
    ///
    /// Panics if the temporary directory cannot be created — a failed fixture
    /// aborts the test loudly, which is the desired mode.
    #[expect(
        clippy::expect_used,
        reason = "tempdir guarantees an absolute path; the expect guards the typed construction only"
    )]
    #[must_use]
    pub fn new() -> Self {
        let dir = tempdir();
        let root =
            MarkedPathBuf::<Absolute>::new(dir.path.as_path()).expect("temp dir path is absolute");
        Self { dir, root }
    }

    /// The absolute root path.
    #[must_use]
    pub fn path(&self) -> &MarkedPathBuf<Absolute> {
        &self.root
    }

    /// The underlying temp directory, for APIs that need to own it.
    #[must_use]
    pub const fn dir(&self) -> &tempfile::TempDir {
        &self.dir.dir
    }

    /// Joins a typed relative path onto the root, producing an absolute path.
    ///
    /// The argument is a [`MarkedPath<Relative>`], so a drive-relative or
    /// absolute path cannot be passed here at all — the type has already
    /// rejected it. The result is likewise absolute by construction.
    #[must_use]
    pub fn join_rel(&self, relative: &MarkedPath<Relative>) -> MarkedPathBuf<Absolute> {
        self.root.join_relative(relative)
    }

    /// Joins a plain relative path onto the root.
    ///
    /// The path is validated as relative before the join, so a drive-prefixed
    /// or absolute argument is refused rather than silently producing a path
    /// that points elsewhere.
    ///
    /// # Errors
    ///
    /// Returns a [`PathError`] if `relative` is not a valid relative path.
    pub fn join_str(&self, relative: &str) -> Result<MarkedPathBuf<Absolute>, PathError> {
        let relative = MarkedPathBuf::<Relative>::new(relative)?;
        Ok(self.join_rel(&relative.as_marked_path()))
    }

    /// The root path as a plain [`PathBuf`], for APIs that take one.
    #[must_use]
    pub fn to_path_buf(&self) -> PathBuf {
        self.root.as_path().to_path_buf()
    }
}

impl std::fmt::Debug for TestRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestRoot")
            .field("path", &self.root.as_path())
            .field("dir", &self.dir.dir.path())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "test code: a failed expect aborts the test loudly, which is the desired mode"
    )]

    use marked_path::MarkedPathAccess;
    use marked_path::MarkedPathBuf;
    use marked_path::Relative;

    use super::TestRoot;

    #[rstest::rstest]
    #[test]
    fn root_is_an_absolute_path() {
        // Given a freshly created test root.

        // When the root is created.
        let root = TestRoot::new();

        // Then its path is absolute.
        assert!(root.path().as_path().is_absolute());
    }

    #[rstest::rstest]
    #[test]
    fn root_carries_the_jinn_test_prefix() {
        // Given a freshly created test root.

        // When the root is created.
        let root = TestRoot::new();

        // Then its directory name carries the jinn test prefix.
        let name = root
            .path()
            .as_path()
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .expect("named test root");
        assert!(name.starts_with(crate::temp::TEST_PREFIX), "got {name:?}");
    }

    #[rstest::rstest]
    #[test]
    fn join_rel_produces_a_path_inside_the_root() {
        // Given a test root and a typed relative path.
        let root = TestRoot::new();
        let relative = MarkedPathBuf::<Relative>::new("notes.txt").expect("relative path");

        // When joining the relative path onto the root.
        let joined = root.join_rel(&relative.as_marked_path());

        // Then the result is absolute and lives inside the root.
        assert!(joined.as_path().is_absolute());
        assert!(joined.as_path().starts_with(root.path().as_path()));
    }

    #[rstest::rstest]
    #[test]
    fn join_rel_keeps_the_relative_tail() {
        // Given a test root and a nested typed relative path.
        let root = TestRoot::new();
        let relative = MarkedPathBuf::<Relative>::new("sub/dir/file.txt").expect("relative path");

        // When joining the relative path onto the root.
        let joined = root.join_rel(&relative.as_marked_path());

        // Then the file name survives the join.
        assert_eq!(
            joined.as_path().file_name(),
            Some(std::ffi::OsStr::new("file.txt"))
        );
    }

    #[rstest::rstest]
    #[test]
    fn join_str_rejects_a_drive_relative_path() {
        // Given a test root.
        let root = TestRoot::new();

        // When joining a drive-relative path onto it.
        let result = root.join_str("C:foo");

        // Then the join is refused.
        assert!(result.is_err());
    }

    #[rstest::rstest]
    #[test]
    fn join_str_rejects_an_absolute_path() {
        // Given a test root.
        let root = TestRoot::new();

        // When joining an absolute path onto it.
        let result = root.join_str("/etc/passwd");

        // Then the join is refused.
        assert!(result.is_err());
    }

    #[rstest::rstest]
    #[test]
    fn join_str_accepts_a_plain_relative_path() {
        // Given a test root.
        let root = TestRoot::new();

        // When joining a plain relative path onto it.
        let joined = root.join_str("plain.txt").expect("joined path");

        // Then the result is absolute and inside the root.
        assert!(joined.as_path().is_absolute());
        assert!(joined.as_path().starts_with(root.path().as_path()));
    }
}
