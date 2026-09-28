//! The per-session task list, as a slice cell.
//!
//! The task list used to live inside `ChatSessionState` as a bare
//! `TaskList`. It is now a cell here — keyed by session id, like the chat
//! log's per-session view state — so the sidebar section, the todo tools,
//! and the picker all resolve it the same way they resolve every other
//! cell.
//!
//! Readers must not grow the map (a session with no entry reads as an
//! empty task list); only writers get-or-insert.

use std::collections::HashMap;

use jinn_core_types::SessionId;
use jinn_slices::SlotKey;

use crate::todo_list::TaskList;

/// The tools cell's task-list payload: one list per session.
pub type TaskLists = HashMap<SessionId, TaskList>;

/// The slot key the per-session task list lives under.
#[must_use]
pub fn task_lists_slot() -> SlotKey {
    SlotKey::builtin("tools", "task_list")
}
