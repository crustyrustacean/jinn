//! State and path resolution for the `@path` file popup.

// The `expect`s below are typed-path operations: re-typing `$HOME`, the cwd,
// or the temp-dir fallback (all absolute by platform contract), or joining a
// bare relative segment onto a base already typed `Absolute`. The panics
// guard programmer error, not runtime state; the invariant is documented
// once here for the family.
#![allow(
    clippy::expect_used,
    clippy::missing_panics_doc,
    reason = "typed-path re-typing and bare-name joins on absolute-by-construction bases"
)]

use marked_path::{Absolute, MarkedPathBuf};

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
/// - Absolute-on-this-platform (the marker's verdict) → itself.
/// - `foo/bar` → `cwd/foo` (the dir containing the path; we list the deepest
///   directory component that ends in `/`, or cwd if none).
///
/// In practice the caller passes the **directory portion** (text up to and
/// including the last `/`), so this is a join against the resolved root.
/// The user's home directory, read once at program startup.
///
/// `resolve_list_dir` resolves `~`, and `~` means the user's home when the
/// program started. Reading it per keystroke would make the answer depend
/// on when it was asked rather than on the user's environment, and would put
/// a process-global read in the middle of an intent handler — so this reads it
/// once, at the boundary where the process environment legitimately belongs,
/// and every later lookup reads this instead.
///
/// `dirs::home_dir()` consults `$HOME` on Unix and the Windows profile
/// directory (`FOLDERID_Profile`) on Windows — the plain-`std` read below the
/// old shape only ever saw `$HOME`, which Windows does not set, silently
/// making the picker's "home" fall back to the cwd there.
#[must_use]
pub fn home_dir() -> MarkedPathBuf<Absolute> {
    let home =
        dirs::home_dir().expect("the user home directory resolves on every supported platform");
    MarkedPathBuf::<Absolute>::new(home).expect("the user home directory is absolute")
}

/// Resolves an autocomplete filter to the directory it names.
///
/// The filter is model/user text. Which filters name an absolute directory
/// is the marker's verdict on the running platform —
/// [`MarkedPathBuf::new`] accepts exactly the texts that are absolute
/// *here* (`/etc` on Unix, `C:\Windows` on Windows) — and everything it
/// refuses is joined against the session cwd. Every produced path is
/// absolute.
#[must_use]
pub fn resolve_list_dir(
    filter: &str,
    cwd: &MarkedPathBuf<Absolute>,
    home: &MarkedPathBuf<Absolute>,
) -> MarkedPathBuf<Absolute> {
    if filter.is_empty() {
        return cwd.clone();
    }
    if let Some(rest) = filter.strip_prefix("~/") {
        return home
            .join(rest)
            .expect("expanding a tilde keeps the home absolute");
    }
    if filter == "~" {
        return home.clone();
    }
    // The filter is user text: let the marker decide if it is absolute on
    // this platform; anything it refuses is relative to the session cwd.
    if let Ok(absolute) = MarkedPathBuf::<Absolute>::new(filter) {
        return absolute;
    }
    cwd.join(filter)
        .expect("joining a relative filter keeps the cwd absolute")
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

    use marked_path::{Absolute, MarkedPathAccess, MarkedPathBuf};

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
        let root = jinn_testutil::tempdir();
        let cwd = root.path.join("proj").expect("temp root joins a bare name");
        let home = root.path.join("u").expect("temp root joins a bare name");

        // When resolving an empty filter.
        let dir = resolve_list_dir("", &cwd, &home);

        // Then the result is the cwd.
        assert_eq!(dir, cwd);
    }

    /// Builds a cwd/home pair from a bare temp-root name, so every resolver
    /// test starts from a portable absolute base.
    fn cwd_and_home(
        root: &jinn_testutil::JinnTempDir,
    ) -> (MarkedPathBuf<Absolute>, MarkedPathBuf<Absolute>) {
        let cwd = root.path.join("proj").expect("temp root joins a bare name");
        let home = root.path.join("u").expect("temp root joins a bare name");
        (cwd, home)
    }

    #[rstest::rstest]
    #[test]
    fn resolve_absolute_filter_returns_path_as_is() {
        // Given a cwd, home, and a filter that names an absolute directory
        // on this platform (`/etc` on Unix, `C:\etc` on Windows) — built
        // from the volume root, never spelled out, so the same code holds
        // on every platform.
        let home_root = jinn_testutil::tempdir();
        let (cwd, home) = cwd_and_home(&home_root);
        let filter_path = jinn_testutil::volume_root()
            .join_all(["etc"])
            .expect("name joins keep the base absolute");
        let filter_text = filter_path.to_string_lossy().into_owned();

        // When resolving the absolute filter (the resolver takes text; the
        // expectation is re-constructed from that same text).
        let dir = resolve_list_dir(&filter_text, &cwd, &home);

        // Then the result is the absolute path the filter names.
        assert_eq!(
            dir,
            MarkedPathBuf::<Absolute>::new(&filter_text).expect("the filter is absolute here")
        );
    }

    /// Pins the Windows resolution of a `/`-prefixed filter.
    ///
    /// `/etc` is *not* absolute on Windows — the marker refuses it — so the
    /// filter joins against the cwd. Per marked-path join semantics an
    /// absolute segment replaces the base, which drive-roots the result
    /// (`D:\etc` for a cwd on `D:`). The pin asserts that resolved shape:
    /// absolute, ending in the `etc` component, and never a panic (the
    /// round-2 Windows-VM failure this module fixed).
    #[cfg(windows)]
    #[rstest::rstest]
    #[test]
    fn resolve_slash_prefixed_filter_joins_relative_on_windows() {
        // Given a temp-rooted cwd/home and a `/`-prefixed filter.
        let home_root = jinn_testutil::tempdir();
        let (cwd, home) = cwd_and_home(&home_root);

        // When resolving the `/`-prefixed filter.
        let dir = resolve_list_dir("/etc", &cwd, &home);

        // Then the result is absolute and drive-rooted into `etc`.
        assert_eq!(
            dir,
            cwd.join("/etc")
                .expect("the marked join of a `/` filter is absolute"),
        );
        // And the result ends with the `etc` component.
        assert_eq!(
            dir.file_name().and_then(|n| n.to_str()),
            Some("etc"),
            "a `/`-prefixed filter on Windows resolves to the drive-rooted `etc` directory"
        );
    }

    #[rstest::rstest]
    #[test]
    fn resolve_tilde_filter_returns_home() {
        // Given a cwd and home.
        let root = jinn_testutil::tempdir();
        let cwd = root.path.join("proj").expect("temp root joins a bare name");
        let home = root.path.join("u").expect("temp root joins a bare name");

        // When resolving a bare tilde.
        let dir = resolve_list_dir("~", &cwd, &home);

        // Then the result is the home dir.
        assert_eq!(dir, home);
    }

    #[rstest::rstest]
    #[test]
    fn resolve_tilde_slash_filter_returns_home_subpath() {
        // Given a cwd and home.
        let root = jinn_testutil::tempdir();
        let cwd = root.path.join("proj").expect("temp root joins a bare name");
        let home = root.path.join("u").expect("temp root joins a bare name");

        // When resolving ~/sub.
        let dir = resolve_list_dir("~/sub", &cwd, &home);

        // Then the result is home/sub.
        assert_eq!(dir, home.join("sub").expect("home joins a bare name"));
    }

    #[rstest::rstest]
    #[test]
    fn resolve_relative_filter_joins_cwd() {
        // Given a cwd and home.
        let root = jinn_testutil::tempdir();
        let cwd = root.path.join("proj").expect("temp root joins a bare name");
        let home = root.path.join("u").expect("temp root joins a bare name");

        // When resolving a relative path `foo/bar`.
        let dir = resolve_list_dir("foo/bar", &cwd, &home);

        // Then the result is cwd/foo/bar.
        assert_eq!(dir, cwd.join("foo/bar").expect("cwd joins a bare name"));
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
