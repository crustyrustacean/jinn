//! Kernel glue for the shared slice route mechanics.
//!
//! The route table and vocabulary live in [`jinn_slices::route`], and
//! [`AppState`]'s implementation of [`SliceActionState`] lives with the type
//! in `jinn-app-state` (the orphan rule requires it). What remains here is the
//! [`IntentResult`] ↔ [`RouteResult`] translation and the one place a
//! slice-requested scope transition is applied.

use jinn_slices::FocusScope;
use jinn_slices::route::{RouteResult, ScopeSignal};

use crate::common::app_state::AppState;
use crate::protocol::intent::IntentResult;

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
