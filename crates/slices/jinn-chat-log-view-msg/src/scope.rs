//! The chat log's own scope.
//!
//! The chat log's keys resolve in a dynamic slice scope: the kernel
//! holds no `Scope::ChatLog` variant and its keymap names none of the
//! log's actions. The scope is navigation-only — every key here drives
//! the cursor or the viewport, none of them capture text — so it
//! reports `Mode::Normal` and input-focused UI stays dark while the
//! log is on top.

use jinn_slices::SliceScopeId;

/// The action name the chat log's `x` key dispatches.
///
/// The kernel's pre-dispatch path needs this name to decide whether an
/// incoming action is the `x` row (which continues the hold-to-ignore
/// sweep) or any other action (which ends it). It is published here so
/// the kernel compares against the same constant the slice dispatches —
/// a slice action name and a kernel intent variant cannot otherwise
/// share a check.
pub const IGNORE_SELECTED_ACTION: &str = "ignore-selected";

/// The scope the chat log's action keys resolve in.
///
/// Built with [`SliceScopeId::navigation`], so it does not capture
/// input: the log's rows bind into the static `Normal` and `Input`
/// scopes where the keymap already resolves, and the id is only the
/// row's *identity* — the half of the `(scope, action)` dispatch key
/// that routes a dynamic intent back to this slice.
#[must_use]
pub fn chat_log_scope() -> SliceScopeId {
    SliceScopeId::navigation("chat-log", "view")
}
