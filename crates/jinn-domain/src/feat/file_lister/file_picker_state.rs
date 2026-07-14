//! State for the `@path` file popup, held on `frontend.file_picker`.

use std::path::PathBuf;

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

/// State for the `@path` file popup, stored on `FrontendState`.
///
/// `entries` and `loading` are written by [`DirectoryListerActor`]; the
/// `selected_index` lives in [`AutocompleteState`](crate::feat::chat_input)
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
    /// `selected_index` from [`AutocompleteState`](crate::feat::chat_input) is
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
/// directory to list, mirroring [`scan_at_paths`](crate::feat::context::prompt_template::scan_at_paths).
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
