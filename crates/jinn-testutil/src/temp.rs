//! Temp-directory wrappers for tests — the only sanctioned way for jinn test
//! code to create a scratch directory.
//!
//! Every directory created here carries the [`TEST_PREFIX`] (`jinn-test-`), so
//! residue from a crashed run (`SIGKILL` runs no destructors) is greppable in
//! the platform temp dir and hand-removable in one glob without ever touching
//! another process's files.
//!
//! Ownership first: these wrappers return an owned [`tempfile::TempDir`] that
//! deletes its directory on drop, like `tempfile` itself. The one exception is
//! [`retained_tempdir`] — spawned actors pin their `Services` fixture for the
//! process lifetime, so such a fixture can never be dropped. Those dirs are
//! created with `keep(true)`, their exact paths are recorded in a registry,
//! and an exit destructor ([`cleanup_retained`], run via `ctor`) removes them
//! when the test binary exits, on any exit path short of `SIGKILL`.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test harness: a failed expect aborts the test loudly, which is the desired mode"
)]

use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;

/// The head every test-created temp directory name must carry.
pub const TEST_PREFIX: &str = "jinn-test-";

/// Exact paths of dirs created by [`retained_tempdir`], removed at process exit.
static RETAINED: LazyLock<parking_lot::Mutex<Vec<PathBuf>>> =
    LazyLock::new(|| parking_lot::Mutex::new(Vec::new()));

/// Creates a prefixed, normally-dropping temp directory.
///
/// # Errors
///
/// Returns the underlying `tempfile` error if directory creation fails.
pub fn tempdir() -> std::io::Result<tempfile::TempDir> {
    tempfile::Builder::new().prefix(TEST_PREFIX).tempdir()
}

/// Creates a prefixed, normally-dropping temp directory under `dir`.
///
/// # Errors
///
/// Returns the underlying `tempfile` error if directory creation fails.
pub fn tempdir_in<P>(dir: P) -> std::io::Result<tempfile::TempDir>
where
    P: AsRef<Path>,
{
    tempfile::Builder::new().prefix(TEST_PREFIX).tempdir_in(dir)
}

/// Creates a prefixed, normally-dropping temp directory with a caller-chosen
/// name head.
///
/// # Panics
///
/// Panics if `prefix` does not start with [`TEST_PREFIX`] — an unprefixed
/// test directory is invisible to the residue conventions and is refused
/// outright rather than created.
///
/// # Errors
///
/// Returns the underlying `tempfile` error if directory creation fails.
pub fn tempdir_with_prefix(prefix: &str) -> std::io::Result<tempfile::TempDir> {
    require_test_prefix(prefix);
    tempfile::Builder::new().prefix(prefix).tempdir()
}

/// Enforces the jinn test prefix, refusing an unprefixed name outright.
///
/// # Panics
///
/// Panics naming the offending prefix.
fn require_test_prefix(prefix: &str) {
    assert!(
        prefix.starts_with(TEST_PREFIX),
        "test temp prefix must start with {TEST_PREFIX:?}, got {prefix:?}"
    );
}

/// Creates a prefixed temp directory that outlives the test body.
///
/// For fixtures that structurally cannot drop — a `Services` handle retained
/// by spawned actors for the process lifetime. The directory is created with
/// `keep(true)` (so a normal drop leaves it in place rather than fighting the
/// actors that still reference paths inside it) and its exact path is
/// registered with [`cleanup_retained`], which removes it when the test
/// binary exits.
///
/// # Errors
///
/// Returns the underlying `tempfile` error if directory creation fails.
pub fn retained_tempdir() -> std::io::Result<tempfile::TempDir> {
    let dir = tempfile::Builder::new()
        .prefix(TEST_PREFIX)
        .disable_cleanup(true)
        .tempdir()?;
    RETAINED.lock().push(dir.path().to_path_buf());
    Ok(dir)
}

/// Builder over `tempfile::Builder` with the jinn test prefix preset.
///
/// The escape hatch for tests needing knobs the free functions don't expose
/// (`keep`, `rand_bytes`). There is deliberately no `prefix` setter — every
/// directory this module produces carries [`TEST_PREFIX`].
#[derive(Debug, Clone)]
pub struct TestTempBuilder {
    inner: tempfile::Builder<'static, 'static>,
}

impl Default for TestTempBuilder {
    fn default() -> Self {
        let mut inner = tempfile::Builder::new();
        inner.prefix(TEST_PREFIX);
        Self { inner }
    }
}

impl TestTempBuilder {
    /// Sets the number of random characters appended to the prefix.
    pub fn rand_bytes(&mut self, n: usize) -> &mut Self {
        self.inner.rand_bytes(n);
        self
    }

    /// Sets whether the directory survives its `TempDir` being dropped.
    pub fn keep(&mut self, keep: bool) -> &mut Self {
        self.inner.disable_cleanup(!keep);
        self
    }

    /// Creates the directory in the platform temp dir.
    ///
    /// # Errors
    ///
    /// Returns the underlying `tempfile` error if directory creation fails.
    pub fn tempdir(&self) -> std::io::Result<tempfile::TempDir> {
        self.inner.tempdir()
    }

    /// Creates the directory under `dir`.
    ///
    /// # Errors
    ///
    /// Returns the underlying `tempfile` error if directory creation fails.
    pub fn tempdir_in<P>(&self, dir: P) -> std::io::Result<tempfile::TempDir>
    where
        P: AsRef<Path>,
    {
        self.inner.tempdir_in(dir)
    }
}

/// A prefixed builder with defaults, for chained customization.
#[must_use]
pub fn builder() -> TestTempBuilder {
    TestTempBuilder::default()
}

/// Removes every registered retained directory. Best-effort; errors ignored.
///
/// Runs once per test-binary exit via a `ctor` destructor. Registered paths
/// are exact — never a glob — so concurrent test binaries (or concurrent
/// worktrees) cannot delete each other's live directories.
pub(crate) fn cleanup_retained() {
    for path in RETAINED.lock().drain(..) {
        let _ = std::fs::remove_dir_all(path);
    }
}

#[ctor::dtor]
fn remove_retained_tempdirs_on_exit() {
    cleanup_retained();
}

/// Reads the registry, for tests in this crate.
#[cfg(test)]
pub(crate) fn retained_paths() -> Vec<PathBuf> {
    RETAINED.lock().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    #[test]
    fn tempdir_uses_the_jinn_test_prefix() {
        // Given the free wrapper.
        // When creating a temp directory.
        let dir = tempdir().expect("temp dir");

        // Then its name carries the jinn test prefix.
        let name = dir
            .path()
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .expect("named temp dir");
        assert!(name.starts_with(TEST_PREFIX), "got {name:?}");
    }

    #[rstest::rstest]
    #[test]
    fn tempdir_in_places_dir_under_given_root() {
        // Given a root directory.
        let root = tempdir().expect("root temp dir");

        // When creating a directory under it.
        let dir = tempdir_in(root.path()).expect("nested temp dir");

        // Then it lands inside the root, still prefixed.
        assert!(dir.path().starts_with(root.path()));
        let name = dir
            .path()
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .expect("named temp dir");
        assert!(name.starts_with(TEST_PREFIX), "got {name:?}");
    }

    #[rstest::rstest]
    #[test]
    #[should_panic(expected = "test temp prefix must start with")]
    fn tempdir_with_prefix_rejects_unprefixed_name() {
        // Given a prefix missing the jinn test head.
        // When creating a directory with it.
        let _ = tempdir_with_prefix("foo");
        // Then creation panics.
    }

    #[rstest::rstest]
    #[test]
    fn tempdir_with_prefix_accepts_prefixed_name() {
        // Given a prefix carrying the jinn test head.
        // When creating a directory with it.
        let dir = tempdir_with_prefix("jinn-test-scratch").expect("temp dir");

        // Then creation succeeds under that exact prefix.
        let name = dir
            .path()
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .expect("named temp dir");
        assert!(name.starts_with("jinn-test-scratch"));
    }

    #[rstest::rstest]
    #[test]
    fn retained_tempdir_registers_its_path() {
        // Given a retained temp directory.
        let dir = retained_tempdir().expect("retained temp dir");

        // Then its exact path sits in the registry.
        assert!(retained_paths().contains(&dir.path().to_path_buf()));
    }

    #[rstest::rstest]
    #[test]
    fn cleanup_removes_registered_paths_only() {
        // Given a registered retained directory and an unregistered kept one.
        let registered = retained_tempdir().expect("retained temp dir");
        let unregistered = builder().keep(true).tempdir().expect("kept temp dir");
        let registered_path = registered.path().to_path_buf();
        let unregistered_path = unregistered.path().to_path_buf();
        assert!(registered_path.exists());
        assert!(unregistered_path.exists());

        // When running the retained cleanup.
        cleanup_retained();

        // Then the registered directory is gone.
        assert!(!registered_path.exists());

        // And the unregistered sibling is untouched (removed here so the
        // test leaves no residue of its own).
        assert!(unregistered_path.exists());
        std::fs::remove_dir_all(&unregistered_path).expect("remove unregistered dir");
    }
}
