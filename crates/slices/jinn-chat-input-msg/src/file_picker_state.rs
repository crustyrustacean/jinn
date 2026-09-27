//! State and path resolution for the `@path` file popup.

use std::path::PathBuf;

use jinn_slices::SlotKey;

/// The slot the `@path` file popup's state is stored under.
///
/// The popup state is a cell rather than a `FrontendState` field: it has
/// one production writer (the directory lister), the rest of its readers
/// are the render pass, and the cell mechanism already carries exactly
/// that shape. The key lives beside the payload it names.
#[must_use]
pub fn file_picker_slot() -> SlotKey {
    SlotKey::builtin("jinn-chat-input", "file-picker")
}

/// One entry in a directory listing.
///
/// Name is the bare entry name (no path prefix). `is_dir` is true for
/// directories so the popup can render a trailing `/` and the confirm flow
/// can descend instead of closing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Bare entry name (e.g. `img.png`, `src`).
    pub name: String,
    /// Whether the entry is a directory.
    pub is_dir: bool,
}

/// State for the `@path` file popup, stored in the
/// [`file_picker_slot`] cell.
///
/// `entries` and `loading` are written by `DirectoryListerActor`; the
/// `selected_index` lives in [`AutocompleteState`](crate::chat_input_state::AutocompleteState)
/// (the popup selection). `expected_request_id` is the staleness guard: only
/// the actor's reply whose `request_id` matches this value is written.
///
/// OWNER: DirectoryListerActor (entries, loading, expected_request_id).
#[derive(Debug, Clone, Default)]
pub struct FilePickerState {
    /// The current directory's entries, unfiltered. Empty until the first
    /// listing arrives. Kept across `/` descents so re-renders are stable.
    pub entries: Vec<FileEntry>,
    /// True while a `ListDirectory` request is in flight. Rendered as
    /// `<loading…>` until the reply lands.
    pub loading: bool,
    /// Monotonic id of the request whose reply we currently expect. The
    /// IntentHandler increments this on every `ListDirectory` emit; the actor
    /// writes its result only when its `request_id` matches this.
    pub expected_request_id: u64,
}

impl FilePickerState {
    /// Builds a state preloaded with entries (test helper / actor write).
    #[must_use]
    pub fn with_entries(entries: Vec<FileEntry>) -> Self {
        Self {
            entries,
            loading: false,
            expected_request_id: 0,
        }
    }

    /// Returns the entries visible for the given `@path` filter.
    ///
    /// The **last path segment** of the filter (the text after the final `/`)
    /// is the filename the user is currently typing. Entries whose name does
    /// not start with that segment are hidden. When the segment is empty (e.g.
    /// `@`, `@foo/`), all entries in the current directory are shown.
    ///
    /// This is the single source of truth for what the popup renders and what
    /// `confirm_at_popup` inserts — render and confirm must agree on the set.
    ///
    /// `selected_index` from
    /// [`AutocompleteState`](crate::chat_input_state::AutocompleteState) is
    /// clamped to this list's length at render and confirm time, so a stale
    /// index (left over from a previous, larger directory) never causes a
    /// no-op confirm or an out-of-range highlight.
    #[must_use]
    pub fn visible_entries(&self, filter: &str) -> Vec<&FileEntry> {
        let segment = filter.rsplit_once('/').map_or(filter, |(_, last)| last);
        self.entries
            .iter()
            .filter(|e| e.name.to_lowercase().starts_with(&segment.to_lowercase()))
            .collect()
    }
}

/// Resolves a raw `@path` filter (the text after `@`) into an absolute
/// directory to list, mirroring `jinn_context::attachment_path::scan_at_paths`.
///
/// - Empty or relative path → `cwd`.
/// - `~` / `~/...` → home.
/// - `/...` → absolute.
/// - `foo/bar` → `cwd/foo` (the dir containing the path; we list the deepest
///   directory component that ends in `/`, or cwd if none).
///
/// In practice the caller passes the **directory portion** (text up to and
/// including the last `/`), so this is a join against the resolved root.
#[must_use]
pub fn resolve_list_dir(filter: &str, cwd: &std::path::Path, home: &std::path::Path) -> PathBuf {
    if filter.is_empty() {
        return cwd.to_path_buf();
    }
    if let Some(rest) = filter.strip_prefix("~/") {
        return home.join(rest);
    }
    if filter == "~" {
        return home.to_path_buf();
    }
    if filter.starts_with('/') {
        return PathBuf::from(filter);
    }
    cwd.join(filter)
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

    use std::path::PathBuf;

    use super::{FileEntry, FilePickerState, resolve_list_dir};

    /// Test helper: a non-directory entry.
    fn file_entry(name: &str) -> FileEntry {
        FileEntry {
            name: name.into(),
            is_dir: false,
        }
    }

    #[rstest::rstest]
    #[test]
    fn resolve_empty_filter_returns_cwd() {
        // Given a cwd and home.
        let cwd = PathBuf::from("/proj");
        let home = PathBuf::from("/home/u");

        // When resolving an empty filter.
        let dir = resolve_list_dir("", &cwd, &home);

        // Then the result is the cwd.
        assert_eq!(dir, cwd);
    }

    #[rstest::rstest]
    #[test]
    fn resolve_absolute_filter_returns_path_as_is() {
        // Given a cwd, home, and an absolute filter.
        let cwd = PathBuf::from("/proj");
        let home = PathBuf::from("/home/u");

        // When resolving an absolute path.
        let dir = resolve_list_dir("/etc", &cwd, &home);

        // Then the result is the absolute path.
        assert_eq!(dir, PathBuf::from("/etc"));
    }

    #[rstest::rstest]
    #[test]
    fn resolve_tilde_filter_returns_home() {
        // Given a cwd and home.
        let cwd = PathBuf::from("/proj");
        let home = PathBuf::from("/home/u");

        // When resolving a bare tilde.
        let dir = resolve_list_dir("~", &cwd, &home);

        // Then the result is the home dir.
        assert_eq!(dir, home);
    }

    #[rstest::rstest]
    #[test]
    fn resolve_tilde_slash_filter_returns_home_subpath() {
        // Given a cwd and home.
        let cwd = PathBuf::from("/proj");
        let home = PathBuf::from("/home/u");

        // When resolving ~/sub.
        let dir = resolve_list_dir("~/sub", &cwd, &home);

        // Then the result is home/sub.
        assert_eq!(dir, PathBuf::from("/home/u/sub"));
    }

    #[rstest::rstest]
    #[test]
    fn resolve_relative_filter_joins_cwd() {
        // Given a cwd and home.
        let cwd = PathBuf::from("/proj");
        let home = PathBuf::from("/home/u");

        // When resolving a relative path `foo/bar`.
        let dir = resolve_list_dir("foo/bar", &cwd, &home);

        // Then the result is cwd/foo/bar.
        assert_eq!(dir, PathBuf::from("/proj/foo/bar"));
    }

    #[rstest::rstest]
    #[test]
    fn visible_entries_returns_all_when_filter_empty() {
        // Given a picker with two entries.
        let picker = FilePickerState::with_entries(vec![
            FileEntry {
                name: "a".into(),
                is_dir: false,
            },
            FileEntry {
                name: "b".into(),
                is_dir: true,
            },
        ]);

        // When filtering with an empty segment.
        let visible = picker.visible_entries("");

        // Then both entries are returned, in stored order.
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].name, "a");
        assert_eq!(visible[1].name, "b");
    }

    #[rstest::rstest]
    #[test]
    fn visible_entries_narrows_by_last_segment() {
        // Given a picker with several entries.
        let picker = FilePickerState::with_entries(vec![
            file_entry("src"),
            file_entry("srv"),
            file_entry("static"),
            file_entry("img.png"),
        ]);

        // When filtering with the prefix "sr".
        let visible = picker.visible_entries("sr");

        // Then only entries starting with "sr" are returned.
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].name, "src");
        assert_eq!(visible[1].name, "srv");
    }

    #[rstest::rstest]
    #[test]
    fn visible_entries_is_case_insensitive() {
        // Given a picker with mixed-case entries.
        let picker = FilePickerState::with_entries(vec![
            file_entry("README.md"),
            file_entry("readme.txt"),
            file_entry("src"),
        ]);

        // When filtering with a lowercase prefix.
        let visible = picker.visible_entries("read");

        // Then both case variants match.
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].name, "README.md");
        assert_eq!(visible[1].name, "readme.txt");
    }

    #[rstest::rstest]
    #[test]
    fn visible_entries_uses_segment_after_last_slash() {
        // Given a picker and a filter that has already descended.
        let picker = FilePickerState::with_entries(vec![
            file_entry("src"),
            file_entry("srv"),
            file_entry("img.png"),
        ]);

        // When filtering with "foo/s" (already inside a deeper directory).
        let visible = picker.visible_entries("foo/s");

        // Then only the segment after the last `/` ("s") narrows the list.
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].name, "src");
        assert_eq!(visible[1].name, "srv");
    }

    #[rstest::rstest]
    #[test]
    fn visible_entries_all_after_trailing_slash() {
        // Given a picker.
        let picker = FilePickerState::with_entries(vec![file_entry("src"), file_entry("img.png")]);

        // When filtering with "foo/" (trailing slash → empty segment).
        let visible = picker.visible_entries("foo/");

        // Then all entries are shown (empty segment matches everything).
        assert_eq!(visible.len(), 2);
    }
}
