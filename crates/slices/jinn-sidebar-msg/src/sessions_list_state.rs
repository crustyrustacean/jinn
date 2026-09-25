//! Sessions-list view vocabulary — the entry model, tree prompt state,
//! and preview cache shared between the kernel's session list logic and
//! the sidebar slice.
//!
//! The kernel owns the list logic (building entries from the session
//! map, reconcile on removal); the sidebar slice owns the section's
//! interactions. Both speak these types.

use std::collections::HashMap;

use jinn_core_types::SessionId;

pub use jinn_session_list::{SessionEntry, SessionEntryKind};

/// The tree action a confirmation prompt was armed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreePromptAction {
    /// Archive the subtree as-is (`A` key).
    Archive,
    /// Tear down the root, then archive the subtree (`X` key).
    TeardownAndArchive,
}

/// The route-table action string for the archive-subtree key (`A`).
///
/// Shared by the sidebar's route row (which mints the
/// [`crate::DynamicIntent`]) and the kernel's archive-tree-prompt
/// interceptor (which re-keys prompts onto these strings), so a rename
/// breaks compilation instead of silently detaching the confirm press.
pub const TREE_ARCHIVE_ACTION: &str = "archive subtree";

/// The route-table action string for the teardown+archive key (`X`).
///
/// See [`TREE_ARCHIVE_ACTION`] for why this is a shared constant.
pub const TREE_TEARDOWN_ACTION: &str = "teardown+archive tree";

/// State of the archive-tree confirmation prompt.
///
/// OWNER: IntentHandler (armed on the first press of the arming key,
/// consumed when that same key is pressed again, dismissed on any other
/// intent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveTreePrompt {
    /// Armed: the subtree was fully idle at arm time; `count` is the visible
    /// subtree size (selection plus descendants).
    Confirm {
        /// Number of sessions the confirm press will archive.
        count: usize,
        /// Which tree action the confirm press will perform.
        action: TreePromptAction,
    },
    /// Blocked: at least one member is busy; nothing will archive.
    Busy,
}

/// History length component of the preview cache key.
type HistoryLen = usize;
/// Content width component of the preview cache key.
type ContentWidth = u16;

/// Cache for session preview popup rendered lines.
///
/// Keyed by `(SessionId, HistoryLen, ContentWidth)` so that:
/// - Switching sessions produces a cache miss (different `SessionId`).
/// - New completed messages produce a cache miss (different `HistoryLen`).
/// - Terminal resize produces a cache miss (different `ContentWidth`).
#[derive(Debug, Default)]
pub struct SessionPreviewCache {
    entries: HashMap<(SessionId, HistoryLen, ContentWidth), Vec<ratatui::text::Line<'static>>>,
}

impl SessionPreviewCache {
    /// Creates a new empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Looks up cached preview lines for the given key.
    pub fn get(
        &self,
        session_id: &SessionId,
        history_len: HistoryLen,
        width: ContentWidth,
    ) -> Option<&Vec<ratatui::text::Line<'static>>> {
        self.entries.get(&(session_id.clone(), history_len, width))
    }

    /// Drops all cached entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Stores preview lines for the given key.
    pub fn insert(
        &mut self,
        session_id: SessionId,
        history_len: HistoryLen,
        width: ContentWidth,
        lines: Vec<ratatui::text::Line<'static>>,
    ) {
        self.entries.insert((session_id, history_len, width), lines);
    }
}
