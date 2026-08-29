//! Archive-tree validation and prompt flow.
//!
//! The `A` key in the sidebar sessions section archives the selected session
//! and all of its descendants. The visible subtree (effective parent edges —
//! exactly what the sidebar displays, forks included) is validated as
//! all-or-nothing: if any member is busy, the whole archive is rejected.
//!
//! Flow: the first press arms a confirmation prompt (or a busy notice), the
//! second press — re-validated by the intent-handler interceptor — emits an
//! [`ArchiveSessionTree`] command for the session actor, which resolves the
//! authoritative closure and performs the archive.
//!
//! [`ArchiveSessionTree`]: crate::feat::session::protocol::ArchiveSessionTree

use std::collections::{HashMap, HashSet, VecDeque};

use crate::common::app_state::AppState;
use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;
use crate::feat::ui::sidebar::sessions::state::{SessionEntry, SessionEntryKind};
use crate::protocol::SessionId;

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

/// Why an archive-tree request can be rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveTreeError {
    /// The sessions section is not focused.
    WrongSection,
    /// No session is selected.
    NoSelection,
    /// The selected entry is not a session.
    NotASession,
    /// At least one member of the subtree is busy (streaming or sending).
    SubtreeBusy,
}

/// Resolves the visible subtree for the archive-tree action: the selected
/// session plus all of its visible descendants, in BFS order (selection
/// first).
///
/// Uses the sidebar's effective parent edges, so the member set is exactly
/// the subtree the user sees — visually reattached children and forks
/// included.
///
/// # Errors
///
/// Returns [`ArchiveTreeError`] if the sessions section is not focused, no
/// session is selected, the selected entry is not a session, or any member of
/// the subtree is busy.
pub fn archive_tree_members(state: &AppState) -> Result<Vec<SessionId>, ArchiveTreeError> {
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;

    // Sessions section must be focused.
    if !matches!(
        state.frontend.scope_stack.sidebar_section(),
        Some(SidebarSectionId::Sessions)
    ) {
        return Err(ArchiveTreeError::WrongSection);
    }

    // A session must be selected.
    let index = state
        .frontend
        .sessions_section
        .selected_index
        .ok_or(ArchiveTreeError::NoSelection)?;

    let entries = sorted_open_sessions(state);
    let entry = entries.get(index).ok_or(ArchiveTreeError::NoSelection)?;
    if entry.kind != SessionEntryKind::Session {
        return Err(ArchiveTreeError::NotASession);
    }
    let root_id = entry.id.clone();

    let children_map = build_children_map(&entries);
    let members = collect_subtree(&root_id, &children_map);

    // All-or-nothing: every member must be idle for the archive to proceed.
    let is_idle = |id: &SessionId| {
        entries
            .iter()
            .find(|entry| &entry.id == id)
            .is_some_and(|entry| entry.is_idle)
    };
    if !members.iter().all(&is_idle) {
        return Err(ArchiveTreeError::SubtreeBusy);
    }

    Ok(members)
}

/// Builds a parent → children map from the flat sidebar entry list.
///
/// Entries carry their effective (visual) parent, so the resulting edges are
/// exactly the tree the sidebar renders.
fn build_children_map(entries: &[SessionEntry]) -> HashMap<SessionId, Vec<SessionId>> {
    let mut children_map: HashMap<SessionId, Vec<SessionId>> = HashMap::new();
    for entry in entries {
        if let Some(parent_id) = &entry.parent_id {
            children_map
                .entry(parent_id.clone())
                .or_default()
                .push(entry.id.clone());
        }
    }
    children_map
}

/// Collects `root` and all of its descendants in BFS order (root first).
///
/// A visited set guards against cycles so corrupt parent chains cannot hang
/// the caller.
fn collect_subtree(
    root: &SessionId,
    children_map: &HashMap<SessionId, Vec<SessionId>>,
) -> Vec<SessionId> {
    let mut members: Vec<SessionId> = Vec::new();
    let mut visited: HashSet<SessionId> = HashSet::new();
    let mut queue: VecDeque<SessionId> = VecDeque::from([root.clone()]);

    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        members.push(id.clone());
        if let Some(children) = children_map.get(&id) {
            queue.extend(children.iter().cloned());
        }
    }
    members
}

/// Handles the first `SidebarSessionArchiveTree` press — arms the prompt.
///
/// Validates the visible subtree, then arms the confirmation prompt with the
/// subtree size (idle subtree) or the busy notice (any member busy). Never
/// emits a command; the intent-handler interceptor performs re-validation and
/// emits the command on the confirm press.
pub fn handle_session_archive_tree_arm(state: &mut AppState) -> crate::protocol::IntentResult {
    match archive_tree_members(state) {
        Ok(members) => {
            state.frontend.archive_tree_prompt = Some(ArchiveTreePrompt::Confirm {
                count: members.len(),
            });
        }
        Err(ArchiveTreeError::SubtreeBusy) => {
            state.frontend.archive_tree_prompt = Some(ArchiveTreePrompt::Busy);
        }
        // No usable selection: no prompt, no command (silent no-op).
        Err(
            ArchiveTreeError::WrongSection
            | ArchiveTreeError::NoSelection
            | ArchiveTreeError::NotASession,
        ) => {}
    }
    crate::protocol::IntentResult::empty()
}

/// Handles the confirmed `SidebarSessionArchiveTree` press — emits the command.
///
/// Called by the intent-handler interceptor after it has re-validated the
/// subtree; `root` is the selection the validation just resolved.
pub fn handle_session_archive_tree_confirm(
    _state: &mut AppState,
    root: SessionId,
) -> crate::protocol::IntentResult {
    use crate::feat::session::protocol::archive_session_tree::ArchiveSessionTree;
    crate::protocol::IntentResult::new_message(ArchiveSessionTree { root })
}
