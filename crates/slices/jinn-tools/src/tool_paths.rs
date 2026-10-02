//! How a tool turns a user-supplied path into one it can touch.
//!
//! Every filesystem tool takes a `path` argument that the model writes, and
//! that path is usually relative — to whatever directory the session's working
//! directory happens to be. Resolving it is the same arithmetic for every tool
//! (`read`, `write`, `edit`, `save_plan`), so it lives here rather than being
//! restated four times: a tool that resolved paths its own way would be a tool
//! that could disagree with its siblings about which file a name refers to.
//!
//! Resolution is purely lexical. It does not canonicalize, expand `~`, or touch
//! the filesystem, so it cannot fail and cannot be surprised by a symlink.

use std::path::{Path, PathBuf};

/// Resolves a tool's `path` argument against the session's working directory.
///
/// An absolute path is returned as-is; a relative one is joined onto `cwd`.
#[must_use]
pub(crate) fn resolve_path(path: &str, cwd: &Path) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_owned()
    } else {
        cwd.join(p)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;

    #[rstest::rstest]
    fn resolve_path_relative() {
        // Given a relative path and a CWD.
        let cwd = Path::new("/home/user/project");

        // When resolving the path against that CWD.
        let resolved = resolve_path("foo/bar.txt", cwd);

        // Then it's joined against CWD.
        assert_eq!(resolved, PathBuf::from("/home/user/project/foo/bar.txt"));
    }

    #[rstest::rstest]
    fn resolve_path_absolute() {
        // Given an absolute path.
        let cwd = Path::new("/home/user/project");

        // When resolving it against that CWD.
        let resolved = resolve_path("/etc/hosts", cwd);

        // Then it's returned as-is.
        assert_eq!(resolved, PathBuf::from("/etc/hosts"));
    }
}
