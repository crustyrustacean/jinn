//! Destination path resolution for an export.
//!
//! Two cases, decided by whether the user typed an argument:
//!
//! - **No argument** — the export lands beside the session, named
//!   `chat-export.html`. If that name is taken, the ladder climbs to
//!   `chat-export-2.html`, `chat-export-3.html`, and so on without an upper
//!   bound, so repeated exports never clobber one another.
//! - **An argument** — used as typed. An absolute path is taken verbatim, a
//!   leading `~` expands to the home directory, and anything else is joined
//!   onto the session's working directory.
//!
//! Resolution touches the filesystem (existence checks) and therefore runs on
//! the blocking worker, never on the render thread. The default filename's
//! extension is HTML because the default export is meant to be opened in a
//! browser; other formats are opt-in by typing their extension.

use std::path::{Path, PathBuf};

use wherror::Error;

use crate::format::DEFAULT_EXTENSION;

/// The stem of the default export filename, before the extension.
const DEFAULT_STEM: &str = "chat-export";

/// Something went wrong deciding where an export should be written.
///
/// Deliberately carries a user-facing message: the actor pushes this text
/// straight into an `Error` chat entry, so it names the concrete problem
/// rather than a bare "export failed".
#[derive(Debug, Error)]
#[error(debug)]
pub struct ResolvePathError {
    /// What the user can do about it, in one sentence.
    pub reason: String,
}

/// The home directory, or an error naming it.
///
/// # Errors
///
/// Returns an error if the home directory cannot be determined, which
/// happens only when `~` is typed and no home directory is set.
fn home_dir() -> Result<PathBuf, ResolvePathError> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| ResolvePathError {
            reason: "cannot expand `~`: no home directory is set".to_owned(),
        })
}

/// Splits a slash-command argument into the command name and its argument.
///
/// Splits on the first run of whitespace, once, so an argument containing
/// internal spaces survives intact: `/export my notes.md` yields
/// `("export", "my notes.md")`. Grapheme-aware, so a multi-byte whitespace
/// character is not split down the middle.
///
/// # Panics
///
/// Never; the function is total.
#[must_use]
pub fn split_command_and_argument(input: &str) -> (&str, &str) {
    let (name, rest) = input
        .trim_start_matches('/')
        .split_once(|c: char| c.is_whitespace())
        .map_or((input.trim_start_matches('/'), ""), |(name, rest)| {
            (name, rest.trim_start())
        });
    (name, rest)
}

/// Resolves the destination path for an export.
///
/// `argument` is the raw text after the command name: empty selects the
/// default ladder, anything else is resolved against `cwd` per the rules in
/// the module docs.
///
/// # Errors
///
/// Returns an error if the argument starts with `~` and no home directory is
/// available to expand it.
pub fn resolve_export_path(argument: &str, cwd: &Path) -> Result<PathBuf, ResolvePathError> {
    if argument.is_empty() {
        return Ok(default_export_path(cwd));
    }
    if argument == "~" {
        return home_dir();
    }
    if let Some(rest) = argument.strip_prefix("~/") {
        let home = home_dir()?;
        return Ok(home.join(rest));
    }
    let candidate = PathBuf::from(argument);
    if candidate.is_absolute() {
        return Ok(candidate);
    }
    Ok(cwd.join(candidate))
}

/// Picks the first free `chat-export*.html` name in `cwd`.
///
/// Starts at `chat-export.html` and climbs to `chat-export-2.html` onward
/// while the candidate exists. There is no upper bound: if the directory is
/// full of exports, this returns a name that does not exist yet, and the
/// subsequent write is what will fail if the directory is genuinely full.
#[must_use]
pub fn default_export_path(cwd: &Path) -> PathBuf {
    let first = cwd.join(format!("{DEFAULT_STEM}.{DEFAULT_EXTENSION}"));
    if !first.exists() {
        return first;
    }
    // The ladder is deliberately unbounded: any number of prior exports must
    // still yield a writable name rather than clobbering one of them.
    #[expect(
        clippy::maybe_infinite_iter,
        reason = "the ladder has no upper bound by design; it stops at the first free name"
    )]
    let free = (2_u32..)
        .map(|n| cwd.join(format!("{DEFAULT_STEM}-{n}.{DEFAULT_EXTENSION}")))
        .find(|candidate| !candidate.exists());
    free.unwrap_or(first)
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
    fn bare_argument_yields_default_filename() {
        // Given a session cwd.
        let dir = tempfile::tempdir().expect("temp dir");

        // When resolving an empty argument.
        let path = resolve_export_path("", dir.path()).expect("resolve");

        // Then the default filename in that cwd is chosen.
        assert_eq!(path, dir.path().join("chat-export.html"));
    }

    #[rstest::rstest]
    fn taken_default_name_yields_numbered_filename() {
        // Given a cwd where the default filename already exists.
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("chat-export.html"), "old").expect("write");

        // When resolving an empty argument.
        let path = resolve_export_path("", dir.path()).expect("resolve");

        // Then the next free numbered name is chosen.
        assert_eq!(path, dir.path().join("chat-export-2.html"));
    }

    #[rstest::rstest]
    fn two_taken_default_names_yields_third() {
        // Given a cwd where the first two ladder names are taken.
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("chat-export.html"), "a").expect("write");
        std::fs::write(dir.path().join("chat-export-2.html"), "b").expect("write");

        // When resolving an empty argument.
        let path = resolve_export_path("", dir.path()).expect("resolve");

        // Then the ladder climbs to the third name.
        assert_eq!(path, dir.path().join("chat-export-3.html"));
    }

    #[rstest::rstest]
    fn absolute_argument_is_unchanged() {
        // Given an absolute destination.
        let dir = tempfile::tempdir().expect("temp dir");
        let target = dir.path().join("elsewhere").join("out.html");

        // When resolving it against a different cwd.
        let path = resolve_export_path(
            target.to_str().expect("utf-8 path"),
            Path::new("/some/other/cwd"),
        )
        .expect("resolve");

        // Then it is used exactly as given.
        assert_eq!(path, target);
    }

    #[rstest::rstest]
    fn relative_argument_joins_the_session_cwd() {
        // Given a relative destination.
        let dir = tempfile::tempdir().expect("temp dir");

        // When resolving it against the session cwd.
        let path = resolve_export_path("notes/session.md", dir.path()).expect("resolve");

        // Then it is joined onto the cwd.
        assert_eq!(path, dir.path().join("notes").join("session.md"));
    }

    #[rstest::rstest]
    fn tilde_argument_expands_to_home() {
        // Given a tilde-prefixed destination.
        let home = home_dir().expect("home dir is set in the test env");

        // When resolving it.
        let path = resolve_export_path("~/exports/out.md", Path::new("/tmp")).expect("resolve");

        // Then it is rooted at the home directory.
        assert_eq!(path, home.join("exports").join("out.md"));
    }

    #[rstest::rstest]
    fn default_path_carries_the_default_extension() {
        // Given a fresh cwd.
        let dir = tempfile::tempdir().expect("temp dir");

        // When picking the default path.
        let path = default_export_path(dir.path());

        // Then its extension is the default format's.
        assert_eq!(
            path.extension().and_then(std::ffi::OsStr::to_str),
            Some(DEFAULT_EXTENSION)
        );
    }

    #[rstest::rstest]
    fn splits_command_from_argument() {
        // Given "/export out.html".
        // When splitting it.
        let (name, argument) = split_command_and_argument("/export out.html");

        // Then the name and argument separate on the first whitespace.
        assert_eq!(name, "export");
        assert_eq!(argument, "out.html");
    }

    #[rstest::rstest]
    fn split_preserves_spaces_inside_the_argument() {
        // Given "/export my notes/session.md".
        let (name, argument) = split_command_and_argument("/export my notes/session.md");

        // When splitting it.
        // Then only the first whitespace run separates; the rest is argument.
        assert_eq!(name, "export");
        assert_eq!(argument, "my notes/session.md");
    }

    #[rstest::rstest]
    fn split_without_argument_yields_empty_argument() {
        // Given "/export".
        let (name, argument) = split_command_and_argument("/export");

        // When splitting it.
        // Then the argument is empty.
        assert_eq!(name, "export");
        assert_eq!(argument, "");
    }

    #[rstest::rstest]
    fn split_tolerates_leading_and_repeated_whitespace() {
        // Given "/export   out.html".
        let (name, argument) = split_command_and_argument("/export   out.html");

        // When splitting it.
        // Then the run of whitespace collapses to one separator.
        assert_eq!(name, "export");
        assert_eq!(argument, "out.html");
    }
}
