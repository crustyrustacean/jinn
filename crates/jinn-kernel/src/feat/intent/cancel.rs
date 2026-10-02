// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! The cancel-stream cascade — stopping a session and everything beneath it.
//!
//! A confirmed cancel is not local. The session's own stream is cancelled
//! inline, and every subagent or attendant below it is stopped too, recursively.
//! Forks are hard boundaries: their descendants are independent threads and
//! out of the cancel's scope.
//!
//! Descendant stops are messages rather than synchronous writes, because each
//! child's phase is owned by its session actor. A caller that owns its own
//! session's phase must drive that phase itself, or a user message it dispatches
//! immediately afterwards will be queued rather than sent.
//!
//! Split out of [`super::handler`] because the cascade walk has more than one
//! caller — `Esc` here, and the attendant slice's manual re-run — and its
//! fork-boundary rule deserves exactly one owner.

use crate::AppState;
use crate::IntentResult;
use crate::protocol::KernelIntent;

pub(crate) fn try_handle_cancel_stream_prompt(
    intent: &KernelIntent,
    state: &mut AppState,
) -> Option<IntentResult> {
    if !state.frontend.cancel_stream_prompt {
        return None;
    }

    // Dismiss the prompt regardless of which intent triggered it.
    state.frontend.cancel_stream_prompt = false;

    if !matches!(intent, KernelIntent::NormalEscape) {
        // Any other key — dismiss prompt, fall through to normal processing.
        return None;
    }

    let session_id = state.session.active_session_id().clone();

    // Check busy state before resetting.
    let was_busy = state.active_session().is_busy();

    // Cancel busy background operations (lifecycle, etc.).
    if was_busy {
        state.active_session_mut().cancel_busy();
    }

    // Cancel stream.
    state.active_session_mut().cancel_stream_and_drain();
    let mut result = IntentResult::empty().with_message(jinn_inference_msg::CancelStream {
        session_id: session_id.clone(),
    });

    // Also cancel any running lifecycle command.
    if was_busy {
        result =
            result.with_message(jinn_session_lifecycle_msg::CancelLifecycleCommand { session_id });
    }

    // The cascade: every subagent or attendant beneath this session stops
    // with it, recursively. Forks are boundaries — their descendants are
    // independent threads, out of the cancel's scope.
    let mut visited = std::collections::HashSet::new();
    visited.insert(state.session.active_session_id().clone());
    result =
        cascade_descendants(state, state.session.active_session_id(), &mut visited).merge(result);

    Some(result)
}

/// Collects the cancel messages for every running descendant of `session_id`.
///
/// Immediate children come from two sources: the in-flight task-spawn
/// registry (subagents) and the live session map (attendants). A child is
/// followed on its origin — `Subagent` and `Attendant` recurse, `Fork` is a
/// hard boundary, `User` is skipped. The `visited` set terminates the walk
/// on a cyclic parent link (the same defence the visible session tree uses).
///
/// Descendant cancels are messages, not synchronous state writes: the
/// session actor owns each child's phase. A caller that also owns its own
/// session's phase must drive it to `Idle` itself, or a user message it
/// dispatches immediately after will be queued rather than sent.
///
/// The walk is shared by every caller that stops a subtree — `Esc` on the
/// active session, and the attendant slice's manual re-run.
#[must_use]
pub fn cascade_descendants(
    state: &AppState,
    session_id: &jinn_core_types::SessionId,
    visited: &mut std::collections::HashSet<jinn_core_types::SessionId>,
) -> IntentResult {
    let mut result = IntentResult::empty();
    let registry = state.task_spawns.clone();

    // Union of both child sources, deduplicated.
    let mut child_ids: Vec<jinn_core_types::SessionId> = registry.children_of(session_id);
    for (id, session) in state.session.iter() {
        if session.parent_session().as_ref() == Some(session_id) && session.is_attendant() {
            child_ids.push(id.clone());
        }
    }
    child_ids.sort();
    child_ids.dedup();

    for child_id in child_ids {
        if !visited.insert(child_id.clone()) {
            continue;
        }
        let child_origin = state
            .try_session(&child_id)
            .map(jinn_session_state::ChatSessionState::origin);
        match child_origin {
            // The child's result is only valid in the context of the parent
            // turn that asked the question — stop it and follow its own
            // subtree.
            Some(
                jinn_session_msg::SessionOrigin::Subagent
                | jinn_session_msg::SessionOrigin::Attendant,
            ) => {
                result = result
                    .with_message(jinn_inference_msg::CancelStream {
                        session_id: child_id.clone(),
                    })
                    .merge(cascade_descendants(state, &child_id, visited));
            }
            // A fork is an independent thread: its own descendants are out
            // of scope. The walk stops here, deliberately. A user-created
            // child is not the cancel's to stop either.
            None
            | Some(jinn_session_msg::SessionOrigin::Fork | jinn_session_msg::SessionOrigin::User) =>
                {}
        }
    }

    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::missing_docs_in_private_items, reason = "test code")]
    use super::*;

    use crate::feat::intent::handler::IntentHandler;
    use jinn_core_types::SessionId;
    use jinn_session_msg::SessionOrigin;
    use jinn_session_state::ChatSessionState;

    /// The slice registry with nothing registered: enough for dispatch tests
    /// that are not exercising composition.
    fn empty_slices() -> jinn_slices::Slices {
        jinn_slices::Slices::new()
    }

    fn empty_routes() -> jinn_slices::route::KeyRoutes {
        jinn_slices::route::KeyRoutes::new()
    }

    fn confirmed_cancel(state: &mut AppState) -> IntentResult {
        state.active_session_mut().begin_streaming();
        state.frontend.cancel_stream_prompt = true;
        IntentHandler::handle(
            &KernelIntent::NormalEscape,
            state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        )
    }

    fn cancel_count(result: &IntentResult) -> usize {
        result
            .message_names
            .iter()
            .filter(|name| name.contains("CancelStream"))
            .count()
    }

    fn link_child(state: &mut AppState, parent_id: &SessionId, origin: SessionOrigin) -> SessionId {
        let parent = state.session.get(parent_id).expect("parent").clone();
        let child = match origin {
            SessionOrigin::Attendant => ChatSessionState::new_attendant(&parent, false),
            _ => {
                let mut child = ChatSessionState::new_child(parent_id, false);
                child.set_origin(origin);
                child
            }
        };
        let child_id = child.session_id().clone();
        state.session.insert(child);
        child_id
    }

    #[rstest::rstest]
    fn confirmed_cancel_stops_immediate_subagents_and_attendants() {
        // Given a parent with a running subagent and a running attendant.
        let mut state = AppState::default_with_scope_focus();
        let parent_id = state.session.active_session_id().clone();
        let subagent = link_child(&mut state, &parent_id, SessionOrigin::Subagent);
        let _attendant = link_child(&mut state, &parent_id, SessionOrigin::Attendant);
        state
            .task_spawns
            .register(parent_id.clone(), subagent.clone());

        // When the confirmed cancel runs.
        let result = confirmed_cancel(&mut state);

        // Then both children receive a cancel (plus the parent's own).
        assert_eq!(cancel_count(&result), 3, "parent + subagent + attendant");
        // And the registry is untouched by the walk itself: it empties when
        // the task future's guard drops, not when the cancel publishes (the
        // registry-layer assertion lives in task_tests).
        assert!(state.task_spawns.has_in_flight(&parent_id));
    }

    #[rstest::rstest]
    fn confirmed_cancel_recurses_through_nested_subagents() {
        // Given a depth-2 subagent tree.
        let mut state = AppState::default_with_scope_focus();
        let root_id = state.session.active_session_id().clone();
        let mid = link_child(&mut state, &root_id, SessionOrigin::Subagent);
        let leaf = link_child(&mut state, &mid, SessionOrigin::Subagent);
        state.task_spawns.register(root_id.clone(), mid.clone());
        state.task_spawns.register(mid.clone(), leaf);

        // When the confirmed cancel runs at the root.
        let result = confirmed_cancel(&mut state);

        // Then every level cancelled — the walk is real recursion.
        assert_eq!(cancel_count(&result), 3, "root + mid + leaf");
    }

    #[rstest::rstest]
    fn confirmed_cancel_stops_at_fork_boundary() {
        // Given a parent with a fork child and a fork grandchild under it.
        let mut state = AppState::default_with_scope_focus();
        let root_id = state.session.active_session_id().clone();
        let fork = link_child(&mut state, &root_id, SessionOrigin::Fork);
        let fork_child = link_child(&mut state, &fork, SessionOrigin::Subagent);
        state.task_spawns.register(fork.clone(), fork_child.clone());

        // When the confirmed cancel runs at the root.
        let result = confirmed_cancel(&mut state);

        // Then only the parent cancelled — the fork subtree is untouched.
        assert_eq!(cancel_count(&result), 1, "parent only; fork is a boundary");
    }

    #[rstest::rstest]
    fn fork_child_subagents_survive_cancelling_grandparent() {
        // Given a fork whose own subagent is running, under a busy root.
        let mut state = AppState::default_with_scope_focus();
        let root_id = state.session.active_session_id().clone();
        let fork = link_child(&mut state, &root_id, SessionOrigin::Fork);
        let fork_subagent = link_child(&mut state, &fork, SessionOrigin::Subagent);
        state
            .task_spawns
            .register(fork.clone(), fork_subagent.clone());

        // When the confirmed cancel runs at the root.
        let _result = confirmed_cancel(&mut state);

        // Then the fork's subagent is still registered as in-flight.
        assert!(
            state.task_spawns.has_in_flight(&fork),
            "the fork's own subagent must survive a grandparent cancel"
        );
    }

    #[rstest::rstest]
    fn single_escape_does_not_cascade() {
        // Given a parent with a running subagent and the prompt NOT armed.
        let mut state = AppState::default_with_scope_focus();
        let parent_id = state.session.active_session_id().clone();
        let subagent = link_child(&mut state, &parent_id, SessionOrigin::Subagent);
        state
            .task_spawns
            .register(parent_id.clone(), subagent.clone());

        // When a single (unconfirmed) escape arrives — over a turn in
        // flight, so arming is allowed.
        state.active_session_mut().begin_streaming();
        let result = IntentHandler::handle(
            &KernelIntent::NormalEscape,
            &mut state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then nothing cancelled — the prompt merely armed.
        assert!(state.frontend.cancel_stream_prompt);
        assert_eq!(cancel_count(&result), 0);
    }

    #[rstest::rstest]
    fn cancel_walk_terminates_on_cyclic_parent_links() {
        // Given two sessions whose parent links form a cycle, every edge
        // reachable through the registry.
        let mut state = AppState::default_with_scope_focus();
        let root_id = state.session.active_session_id().clone();
        let a = link_child(&mut state, &root_id, SessionOrigin::Subagent);
        let b = link_child(&mut state, &a, SessionOrigin::Subagent);
        // Close the cycle: a's parent becomes b — and register both edges so
        // the walk would loop without the visited guard.
        state
            .session
            .get_mut(&a)
            .expect("a")
            .set_parent_session(b.clone());
        state.task_spawns.register(root_id.clone(), a.clone());
        state.task_spawns.register(a.clone(), b.clone());
        state.task_spawns.register(b.clone(), a.clone());

        // When the confirmed cancel runs at the root.
        let result = confirmed_cancel(&mut state);

        // Then the walk terminated (root + a + b, no repeat) — reaching here
        // at all proves termination; the count proves no double-cancel.
        assert_eq!(cancel_count(&result), 3);
    }
}
