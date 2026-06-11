//! Sweep continuation for the ignore-selected intent.
//!
//! When the user holds `x`, each keypress applies the _captured_ context-override
//! (not a toggle) to the next eligible entry, skipping pinned entries and collapsed
//! ignored blocks. The sweep loop runs within a single keypress invocation so that
//! obstacles are transparent — the user never sees a "skip" as a separate step.

use crate::common::app_state::AppState;
use crate::feat::context::protocol::event::ContextOverrideChanged;
use crate::feat::session::chat_entry::ChatEntry;
use crate::feat::session_lifecycle::protocol::command::PersistSession;
use crate::protocol::{ChatEntryId, Command, ContextOverride, Event, IntentResult, SessionId};

use super::intent::advance_selection_one;

/// Execute a full sweep invocation: apply the captured `target` override,
/// skipping pinned entries and collapsed blocks, until an eligible entry is
/// found or the bottom is reached.
///
/// This replaces the old inline loop in `handle_ignore_selected`.
pub(crate) fn run_sweep(state: &mut AppState, target: ContextOverride) -> IntentResult {
    let session_id = state.active_session().session_id().clone();
    let mut changed_ids: Vec<ChatEntryId> = Vec::new();

    loop {
        let session = state.active_session_mut();

        if session.selected_entry_index().is_none() {
            return IntentResult::empty();
        }

        // Skip pinned entries and collapsed blocks.
        let is_pinned = session.selected_entry().is_some_and(ChatEntry::is_pinned);
        if is_pinned || session.is_selected_collapsed_block() {
            if !advance_selection_one(session) {
                if changed_ids.is_empty() {
                    return IntentResult::empty();
                }
                return finalize_sweep(state, session_id, changed_ids);
            }
            continue;
        }

        // Apply the captured state directly (not a toggle).
        if let Some(id) = session.set_entry_context_override(target) {
            changed_ids.push(id);
        }
        session.rebuild_visual_items();

        // Rebuild may have moved the cursor onto a collapsed block.
        if session.selected_entry().is_none() {
            if !advance_selection_one(session) {
                session.set_ignore_sweep(target);
                return finalize_sweep(state, session_id, changed_ids);
            }
            // Landed on another obstacle — loop will skip it.
            continue;
        }

        let Some(selected) = session.selected_entry() else {
            session.set_ignore_sweep(target);
            return finalize_sweep(state, session_id, changed_ids);
        };
        let entry_id = selected.id.clone();

        // Propagate shown state to new sub-blocks when un-ignoring.
        if matches!(
            target,
            ContextOverride::Default | ContextOverride::ForcedInclude
        ) {
            session.propagate_shown_on_unignore(&entry_id);
        }

        // Advance cursor for next keypress.
        let at_bottom = !advance_selection_one(session);
        session.set_ignore_sweep(target);

        if at_bottom {
            return finalize_sweep(state, session_id, changed_ids);
        }
        return finalize_sweep(state, session_id, changed_ids);
    }
}

/// Common tail for the x-sweep: persist session and emit one
/// `ContextOverrideChanged` event per entry whose override actually changed.
fn finalize_sweep(
    _state: &mut AppState,
    session_id: SessionId,
    changed_ids: Vec<ChatEntryId>,
) -> IntentResult {
    let events = changed_ids.into_iter().map(|id| ContextOverrideChanged {
        session_id: session_id.clone(),
        entry_id: id,
    });
    IntentResult::with_message(PersistSession {
        session_id: session_id.clone(),
    })
    .with_messages(events)
}
