//! Archive-tree validation, prompting, and command flow.

use std::collections::{HashMap, HashSet, VecDeque};

use jinn_core_types::SessionId;
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::protocol::IntentResult;
use jinn_session_lifecycle_msg::TeardownSessionTree;
use jinn_session_store_msg::ArchiveSessionTree;
pub use jinn_sidebar_msg::{ArchiveTreePrompt, TreePromptAction};

use super::state::{SessionEntry, SessionEntryKind, mark_in_flight, sorted_open_sessions};

/// Why an archive-tree request can be rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveTreeError {
    /// The sessions section is not focused.
    WrongSection,
    /// No session is selected.
    NoSelection,
    /// The selected entry is not a session.
    NotASession,
    /// At least one member of the subtree is busy.
    SubtreeBusy,
}

/// Resolves the selected session's visible descendants in breadth-first order.
///
/// # Errors
///
/// Returns [`ArchiveTreeError`] when the selection is invalid or any member
/// of the visible subtree is busy.
pub fn archive_tree_members(state: &AppState) -> Result<Vec<SessionId>, ArchiveTreeError> {
    if !matches!(
        state.frontend.sidebar_section(),
        Some(jinn_sidebar_msg::SidebarSectionId::Sessions)
    ) {
        return Err(ArchiveTreeError::WrongSection);
    }
    let index = state
        .frontend
        .with_sections(|sections| sections.sessions.selected_index, || None)
        .ok_or(ArchiveTreeError::NoSelection)?;
    let entries = sorted_open_sessions(state);
    let root = entries.get(index).ok_or(ArchiveTreeError::NoSelection)?;
    if root.kind != SessionEntryKind::Session {
        return Err(ArchiveTreeError::NotASession);
    }
    let members = collect_subtree(&root.id, &build_children_map(&entries));
    if !members.iter().all(|id| {
        entries
            .iter()
            .find(|entry| &entry.id == id)
            .is_some_and(|entry| entry.is_idle)
    }) {
        return Err(ArchiveTreeError::SubtreeBusy);
    }
    Ok(members)
}

/// Arms the prompt on first press, or revalidates and emits the command on
/// the matching second press. Any other tree key dismisses an armed prompt.
pub fn handle_session_tree_action_arm(
    state: &mut AppState,
    action: TreePromptAction,
) -> IntentResult {
    match state.frontend.archive_tree_prompt.as_ref() {
        Some(ArchiveTreePrompt::Confirm { action: armed, .. }) if *armed == action => {
            state.frontend.archive_tree_prompt = None;
            match archive_tree_members(state) {
                Ok(members) => emit_tree_command(state, action, &members),
                Err(ArchiveTreeError::SubtreeBusy) => {
                    state.frontend.archive_tree_prompt = Some(ArchiveTreePrompt::Busy);
                    IntentResult::empty()
                }
                Err(_) => IntentResult::empty(),
            }
        }
        Some(ArchiveTreePrompt::Confirm { .. }) => {
            state.frontend.archive_tree_prompt = None;
            match archive_tree_members(state) {
                Ok(members) => {
                    state.frontend.archive_tree_prompt = Some(ArchiveTreePrompt::Confirm {
                        count: members.len(),
                        action,
                    });
                    IntentResult::empty()
                }
                Err(ArchiveTreeError::SubtreeBusy) => {
                    state.frontend.archive_tree_prompt = Some(ArchiveTreePrompt::Busy);
                    IntentResult::empty()
                }
                Err(_) => IntentResult::empty(),
            }
        }
        Some(ArchiveTreePrompt::Busy) => {
            state.frontend.archive_tree_prompt = None;
            match archive_tree_members(state) {
                Ok(members) => emit_tree_command(state, action, &members),
                Err(ArchiveTreeError::SubtreeBusy) => {
                    state.frontend.archive_tree_prompt = Some(ArchiveTreePrompt::Busy);
                    IntentResult::empty()
                }
                Err(_) => IntentResult::empty(),
            }
        }
        None => match archive_tree_members(state) {
            Ok(members) => {
                state.frontend.archive_tree_prompt = Some(ArchiveTreePrompt::Confirm {
                    count: members.len(),
                    action,
                });
                IntentResult::empty()
            }
            Err(ArchiveTreeError::SubtreeBusy) => {
                state.frontend.archive_tree_prompt = Some(ArchiveTreePrompt::Busy);
                IntentResult::empty()
            }
            Err(_) => IntentResult::empty(),
        },
    }
}

/// Emits a previously validated tree command.
///
/// `members` is the closure the prompt was confirmed against, so the tint
/// covers every member and not just the root.
pub fn handle_session_tree_action_confirm(
    state: &mut AppState,
    action: TreePromptAction,
    members: &[SessionId],
) -> IntentResult {
    state.frontend.archive_tree_prompt = None;
    emit_tree_command(state, action, members)
}

/// Marks every member in flight, then builds the tree command for `action`.
///
/// Taking the members here means no dispatch path can emit a command without
/// also tinting the sessions it disposes of.
fn emit_tree_command(
    state: &AppState,
    action: TreePromptAction,
    members: &[SessionId],
) -> IntentResult {
    mark_in_flight(state, members);
    command_for(action, members[0].clone())
}

fn command_for(action: TreePromptAction, root: SessionId) -> IntentResult {
    match action {
        TreePromptAction::Archive => IntentResult::new_message(ArchiveSessionTree { root }),
        TreePromptAction::TeardownAndArchive => {
            IntentResult::new_message(TeardownSessionTree { root })
        }
    }
}

fn build_children_map(entries: &[SessionEntry]) -> HashMap<SessionId, Vec<SessionId>> {
    entries
        .iter()
        .filter_map(|entry| {
            entry
                .parent_id
                .as_ref()
                .map(|parent_id| (parent_id.clone(), entry.id.clone()))
        })
        .fold(
            HashMap::<SessionId, Vec<SessionId>>::new(),
            |mut children, (parent_id, child_id)| {
                children.entry(parent_id).or_default().push(child_id);
                children
            },
        )
}

fn collect_subtree(
    root: &SessionId,
    children_map: &HashMap<SessionId, Vec<SessionId>>,
) -> Vec<SessionId> {
    let mut members = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::from([root.clone()]);
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
