//! Kernel glue for the shared slice route mechanics.
//!
//! The route table and vocabulary live in [`jinn_slices::route`]. This module
//! provides only the kernel-side [`KernelIntent`] → [`EditIntent`] translation
//! and [`AppState`]'s implementation of [`SliceActionState`].

use jinn_slices::FocusScope;
use jinn_slices::route::{EditIntent, RouteResult, ScopeSignal, SliceActionState};

use crate::common::app_state::AppState;
use crate::protocol::intent::IntentResult;
use crate::protocol::intent::KernelIntent;

impl SliceActionState for AppState {
    fn active_session_title(&self) -> Option<String> {
        self.active_session().title().map(str::to_owned)
    }

    fn active_session_id(&self) -> jinn_core_types::SessionId {
        self.session.active_session_id().clone()
    }

    fn push_session_error(&mut self, message: &str) {
        self.active_session_mut()
            .push_entry(crate::protocol::ChatEntry::error(message));
    }

    fn active_session_cwd(&self) -> std::path::PathBuf {
        self.active_session().cwd().to_owned()
    }

    fn publish_session_cwd(
        &self,
        session_id: jinn_core_types::SessionId,
        cwd: std::path::PathBuf,
    ) -> jinn_slices::PublishClosure {
        crate::common::bridge::Bridge::publish_closure(jinn_session_lifecycle_msg::SetSessionCwd {
            session_id,
            cwd,
        })
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// Translates the kernel's editing intents into the slice-hook
/// vocabulary.
///
/// `None` means the intent is not an editing surface action — hooks are
/// never consulted for it.
#[must_use]
pub fn as_edit_intent(intent: &KernelIntent) -> Option<EditIntent> {
    match intent {
        KernelIntent::InsertChar { ch } => Some(EditIntent::InsertChar(*ch)),
        KernelIntent::DeleteGrapheme => Some(EditIntent::DeleteBackward),
        KernelIntent::DeleteGraphemeForward => Some(EditIntent::DeleteForward),
        KernelIntent::MoveCursorLeft => Some(EditIntent::CursorLeft),
        KernelIntent::MoveCursorRight => Some(EditIntent::CursorRight),
        KernelIntent::MoveCursorToStart => Some(EditIntent::CursorHome),
        KernelIntent::MoveCursorToEnd => Some(EditIntent::CursorEnd),
        KernelIntent::PasteText { text } => Some(EditIntent::Paste(text.clone())),
        _ => None,
    }
}

/// Converts the kernel's [`IntentResult`] into the slice-level
/// [`RouteResult`] (they are the same shape; this erases the alias).
#[must_use]
pub fn into_route_result(result: IntentResult) -> RouteResult {
    RouteResult {
        messages: result.messages,
        message_names: result.message_names,
        scope_signal: result.scope_signal,
    }
}

/// Converts a slice-level [`RouteResult`] back into the kernel's
/// [`IntentResult`] alias.
#[must_use]
pub fn from_route_result(result: RouteResult) -> IntentResult {
    IntentResult {
        messages: result.messages,
        message_names: result.message_names,
        scope_signal: result.scope_signal,
    }
}

/// Applies a route action's scope transition to the scope stack.
///
/// The handler is the exempt scope-stack writer; this is the only
/// place a slice-requested transition lands.
pub fn apply_scope_signal(result: &mut IntentResult, state: &mut AppState) {
    let Some(signal) = result.scope_signal.take() else {
        return;
    };
    match signal {
        ScopeSignal::Push(id) => state.frontend.scope_push(FocusScope::Dynamic(id)),
        ScopeSignal::PopIf(id) => {
            if matches!(&state.frontend.scope(), FocusScope::Dynamic(cur) if *cur == id) {
                state.frontend.scope_pop();
            }
        }
    }
}
