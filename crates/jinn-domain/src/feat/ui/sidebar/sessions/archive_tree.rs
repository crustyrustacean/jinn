//! Archive-tree confirmation prompt state.
//!
//! The `A` key in the sidebar sessions section archives the selected session
//! and all of its descendants behind a press-again confirmation. This module
//! holds the prompt state the `IntentHandler` arms and consumes; validation
//! and the command emission live here too (see below).

/// State of the archive-tree confirmation prompt.
///
/// OWNER: IntentHandler (armed on the first `SidebarSessionArchiveTree`,
/// consumed on the second `SidebarSessionArchiveTree` or dismissed on any
/// other intent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveTreePrompt {
    /// Armed: the subtree was fully idle at arm time; `count` is the visible
    /// subtree size (selection plus descendants).
    Confirm {
        /// Number of sessions the confirm press will archive.
        count: usize,
    },
    /// Blocked: at least one member is busy; nothing will archive.
    Busy,
}
