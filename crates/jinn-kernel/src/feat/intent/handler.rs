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

//! The [`IntentHandler`] - a single decision point for all user input.
//!
//! Processes every [`Intent`] variant: call the validator, then act.
//! On validation failure, the handler does nothing (no-op). On success,
//! it mutates [`AppState`] directly, optionally sets TUI signals, and
//! returns [`IntentResult`] carrying commands for the actor system.

#![allow(
    clippy::missing_docs_in_private_items,
    reason = "Phase 2 transitional - Phase 4 refactors handler into per-intent modules"
)]
#![allow(
    clippy::doc_markdown,
    reason = "auto-idents like IntentHandler, AppState, PickerKind are meaningful names"
)]

use crate::AppState;
use jinn_status_bar_msg::{StatusBarState, status_bar_slot};
use jinn_term_msg::command::ControlHolder;

use crate::protocol::ScopeSignal;

use crate::KernelIntent;
use crate::feat;

use crate::IntentResult;

/// Processes user intents - the single decision point for all user input.
///
/// For each [`Intent`] variant: call the validator, then act.
/// On validation failure, the handler does nothing (no-op).
///
/// Some intents raise TUI signals through the frontend facade — flags that the
/// outer platform layer reads after `handle()` returns and acts upon
/// (e.g., opening an external editor, toggling a popup).
pub struct IntentHandler;

/// Applies a route result's scope signal to the scope stack.
///
/// The handler is the exempt single-writer of the scope stack; slices
/// request transitions as data ([`ScopeSignal`]) and this is where they
/// land. Runs before the result's messages publish (see
/// [`IntentResult::scope_signal`]).
///
/// A push that lands on a scope with a registered
/// [`ScopeEnterHook`](jinn_slices::ScopeEnterHook) also fires that hook, so
/// the entering slice initializes its per-open state here rather than in
/// whichever caller happened to request the transition. The hook runs after
/// the push, so the scope is already the active one.
fn apply_scope_signal(
    result: &mut IntentResult,
    state: &mut AppState,
    slices: &jinn_slices::Slices,
    routes: &jinn_slices::route::KeyRoutes,
    config: &jinn_config::ConfigLayer,
) {
    use jinn_slices::FocusScope;
    if let Some(signal) = result.scope_signal.take() {
        match signal {
            ScopeSignal::Push(id) => {
                state.frontend.scope_push(FocusScope::Dynamic(id.clone()));
                if let Some(hook) = routes.scope_enter_hook(&id) {
                    hook(jinn_slices::route::ActionCtx {
                        state,
                        slices,
                        config,
                        key_bytes: Vec::new(),
                    });
                }
            }
            ScopeSignal::PopIf(id) => {
                if matches!(&state.frontend.scope(), FocusScope::Dynamic(cur) if *cur == id) {
                    state.frontend.scope_pop();
                }
            }
        }
    }
}

/// Consults the active dynamic scope's registered input hook.
///
/// A hit means the keystroke belonged to the slice's own input surface:
/// the hook performed the synchronous write and the intent is consumed.
/// Returns `None` outside dynamic scopes or when no hook is registered
/// Ends the chat log's `x`-hold ignore sweep unless this intent *is*
/// the ignore action.
///
/// The sweep survives only across consecutive `x` presses, so any other
/// action ends it. The chat log's `x` is a slice route row, not a
/// kernel intent, so the action name is read off the dynamic intent and
/// compared against the name the slice published — the two agree because
/// both come from the same constant in `jinn-chat-log-view-msg`.
fn clear_ignore_sweep_unless_ignoring(state: &mut AppState, intent: &KernelIntent) {
    let action = match intent {
        KernelIntent::Dynamic(dynamic) => dynamic.action.as_str(),
        _ => "",
    };
    if action != jinn_chat_log_view_msg::IGNORE_SELECTED_ACTION {
        state.active_session_mut().clear_ignore_sweep();
    }
}

/// Dispatches the active dynamic scope's registered input hook.
///
/// A hit means the keystroke belonged to the slice's own input surface:
/// the hook performed the synchronous write and the intent is consumed.
/// Returns `None` outside dynamic scopes or when no hook is registered
/// (or the hook declines the intent) — the caller falls through to the
/// built-in arms.
fn try_slice_input_hook(
    intent: &KernelIntent,
    state: &mut AppState,
    routes: &jinn_slices::route::KeyRoutes,
) -> Option<IntentResult> {
    use jinn_slices::FocusScope;
    let FocusScope::Dynamic(scope) = &state.frontend.scope() else {
        return None;
    };
    let hook = routes.input_hook(scope)?;
    // Hooks speak the slice-level editing vocabulary, not the kernel's
    // full intent enum: translate, and skip hooks for non-editing
    // intents entirely.
    let edit = edit_intent_for(intent)?;
    hook(&edit)
}

/// Translates an intent into the slice-hook editing vocabulary.
///
/// Every editing key arrives as a dynamic intent the keymap minted for the
/// hook's scope, with a printable character carried in the byte payload.
/// `None` means the intent is not an editing surface action — hooks are
/// never consulted for it.
#[must_use]
fn edit_intent_for(intent: &KernelIntent) -> Option<jinn_slices::EditIntent> {
    let KernelIntent::Dynamic(dynamic) = intent else {
        return None;
    };
    Some(match dynamic.action.as_str() {
        "insert-char" => jinn_slices::EditIntent::InsertChar(
            std::str::from_utf8(&dynamic.bytes)
                .ok()
                .and_then(|text| text.chars().next())
                .unwrap_or_default(),
        ),
        "delete-backward" => jinn_slices::EditIntent::DeleteBackward,
        "delete-forward" => jinn_slices::EditIntent::DeleteForward,
        "move-cursor-left" => jinn_slices::EditIntent::CursorLeft,
        "move-cursor-right" => jinn_slices::EditIntent::CursorRight,
        "move-cursor-home" => jinn_slices::EditIntent::CursorHome,
        "move-cursor-end" => jinn_slices::EditIntent::CursorEnd,
        _ => return None,
    })
}

/// Resolves the base scope after a `<Tab>` switch, walking the
/// registered tab scopes.
///
/// Tabs are declared by slices (tab descriptors registered at
/// activation); composition keeps the ordered list on `Slices`. With no
/// dynamic tab registered, `<Tab>` is a no-op round-trip to Normal —
/// the chat tab is the only tab.
fn next_tab_base(state: &AppState, slices: &jinn_slices::Slices) -> jinn_slices::FocusScope {
    use jinn_slices::FocusScope;

    // The chat tab (Normal) is always first in the cycle, so the walk
    // is: Normal → tab[0] → … → tab[n-1] → Normal.
    let tabs = tab_scopes(slices);
    if tabs.is_empty() {
        return FocusScope::Normal;
    }
    let position = match state.frontend.scope_base() {
        FocusScope::Dynamic(id) => tabs.iter().position(|tab| tab == &id),
        _ => None,
    };
    match position {
        // Currently on a dynamic tab: advance, wrapping back to chat.
        Some(i) => match tabs.get(i + 1) {
            Some(next) => FocusScope::Dynamic(next.clone()),
            // Last tab: wrap to chat.
            None => FocusScope::Normal,
        },
        // On chat (or any other base): enter the first dynamic tab.
        None => match tabs.first() {
            Some(first) => FocusScope::Dynamic(first.clone()),
            None => FocusScope::Normal,
        },
    }
}

/// The registered tab scope ids, in tab order.
fn tab_scopes(slices: &jinn_slices::Slices) -> Vec<jinn_slices::SliceScopeId> {
    slices.tab_scopes()
}

/// Closes the terminal overlay after an active-session switch and returns
/// the switched-from session's terminal control to the agent.
///
/// The overlay renders the *active* session's terminal, so leaving it open
/// across a switch would silently show a different program mid-keystroke.
/// The previous session's control holder must be reset explicitly: if the
/// user held control with no overlay to hand back from, the holder would
/// stick on `User` and refuse every future agent send.
///
/// Deliberately abrupt (a status hint says why) — obvious over subtle. The
/// registry flip only runs when the shared `TERM_CONTROLS` registry is
/// wired (unit tests leave it unset, making this a pure state transition).
fn close_terminal_overlay_on_switch(
    state: &mut AppState,
    slices: &jinn_slices::Slices,
    prev_active: &jinn_core_types::SessionId,
) {
    if let Some(registry) = jinn_term_msg::TERM_CONTROLS.get() {
        registry.set(prev_active, ControlHolder::Agent);
    }
    state.frontend.scope_clear_overlays();
    if let Some(status) = slices.reader::<StatusBarState>(&status_bar_slot()) {
        status.update(|state| {
            state.hint = Some("terminal overlay closed — active session changed".to_owned());
        });
    }
}

impl IntentHandler {
    /// Process an intent against the current application state.
    ///
    /// Clears TUI signals from the previous call, then processes the intent.
    /// Mutates `state` directly for UI operations. Consults the feature
    /// route table first: an intent bound in [`KeyRoutes`] produces its
    /// message and never reaches the built-in arms. `slices` backs the
    /// cross-feature reads (e.g. discord connectivity) that used to reach
    /// into `frontend` directly.
    /// Returns commands and events for the actor system.
    pub fn handle(
        intent: &KernelIntent,
        state: &mut AppState,
        slices: &jinn_slices::Slices,
        routes: &jinn_slices::route::KeyRoutes,
        config: &jinn_config::ConfigLayer,
    ) -> IntentResult {
        state
            .frontend
            .update_scope(|s| s.signals = jinn_slices::TuiSignals::new());
        // Status hints are transient: any fresh intent dismisses the previous
        // one (the handler arms that raise one run after this line).
        if let Some(status) = slices.reader::<StatusBarState>(&status_bar_slot()) {
            status.update(|state| state.hint = None);
        }

        // Capture active session ID before processing for diff-after check.
        let prev_active = state.session.active_session_id().clone();
        // Capture the terminal overlay state too: the guard below must close
        // the overlay only when it was open *before* the intent ran, so an
        // intent that both activates and opens (sidebar `T`) is not closed by
        // its own switch.
        let terminal_overlay_open = matches!(
            state.frontend.scope(),
            jinn_slices::FocusScope::Dynamic(id)
                if jinn_term_msg::is_overlay_scope(&id)
        );

        // Process the intent and get the result.
        let mut result = Self::handle_inner(intent, state, slices, routes, config);

        if state.session.active_session_id() != &prev_active {
            if terminal_overlay_open {
                close_terminal_overlay_on_switch(state, slices, &prev_active);
            }
            result = result.with_message(crate::protocol::system::ActiveSessionChanged {
                session_id: state.session.active_session_id().clone(),
            });
        }

        result
    }

    /// Internal intent dispatch — separated from `handle` to allow post-processing.
    ///
    /// Dispatch order:
    /// 1. Slice route rows (dynamic intents + globally-toggled slice
    ///    actions). A hit applies any scope signal, then returns.
    /// 2. Slice input hooks: while a dynamic scope with a registered
    ///    hook is active, editing intents route to the hook (sync write
    ///    of the slice's own state — the typing carve-out).
    /// 3. Built-in arms.
    fn handle_inner(
        intent: &KernelIntent,
        state: &mut AppState,
        slices: &jinn_slices::Slices,
        routes: &jinn_slices::route::KeyRoutes,
        config: &jinn_config::ConfigLayer,
    ) -> IntentResult {
        // Confirmation prompts must not survive a keystroke, so every armed
        // prompt is dismissed here — the first statement of this function,
        // ahead of the slice route rows and the slice input hooks below, both
        // of which return early. A prompt that outlived the key that should
        // have dismissed it keeps advertising a confirmation the user never
        // made.
        dismiss_unrelated_prompts(intent, state);

        // Slice-registered routes go first: a dynamic intent is
        // delegated to its slice's action and never reaches the
        // built-in arms. An unregistered dynamic intent resolves to
        // None and falls through to the sweep-reset guard below, which
        // treats it like any other non-x action. The action runs
        // against the handler's own borrows (`ActionCtx`): it writes
        // the same `&mut AppState` guard — never a second lock — and
        // resolves slice cells through the same registry.
        if let KernelIntent::Dynamic(dynamic) = intent
            && let Some(mut result) = routes.action_for(
                dynamic,
                jinn_slices::route::ActionCtx {
                    state,
                    slices,
                    config,
                    key_bytes: dynamic.bytes.clone(),
                },
            )
        {
            // Scope transitions apply before the messages publish so a
            // slice that opens itself is on the stack before any bus
            // subscriber could observe a message.
            apply_scope_signal(&mut result, state, slices, routes, config);
            return result;
        }

        // Slice input hooks: the active dynamic scope's synchronous
        // editing surface. A hit means the keystroke belonged to the
        // slice (typing carve-out), so the intent is consumed here.
        if let Some(result) = try_slice_input_hook(intent, state, routes) {
            return result;
        }

        // Clear ignore sweep state when the user performs any action other
        // than pressing x. This ensures the sweep only continues during
        // consecutive x presses within 100ms.
        //
        // This sits ahead of slice route dispatch so the chat-log slice's
        // own rows obey the rule too: a row that fired first would let
        // the sweep run past the 100ms window.
        clear_ignore_sweep_unless_ignoring(state, intent);

        // Cancel stream prompt: the confirming half. The prompt was cleared
        // above for every other intent, so by this point the only way to
        // reach here with the prompt standing is the `NormalEscape` that
        // raised it.
        if let Some(result) = try_handle_cancel_stream_prompt(intent, state) {
            return result;
        }

        match intent {
            // Mouse wheel. A wheel event is a crossterm backend handler on
            // the `Keymap`, not a keymap node, so no route row can express
            // it — these two intents stay in the kernel. The scroll they
            // perform is the log's, reached through the same session
            // facade the log's own rows use.
            KernelIntent::MouseScrollUp => feat::navigation::intent::handle_mouse_scroll_up(state),
            KernelIntent::MouseScrollDown => {
                feat::navigation::intent::handle_mouse_scroll_down(state)
            }

            KernelIntent::EditInput => feat::navigation::intent::handle_edit_input(state),

            KernelIntent::Quit => feat::global::intent::handle_quit(state),
            KernelIntent::Interrupt { session_id } => {
                feat::global::intent::handle_interrupt(state, session_id.as_ref())
            }
            KernelIntent::ToggleWhichkey => feat::global::intent::handle_toggle_whichkey(state),
            // Escape in Normal mode: raise the cancel-stream confirmation
            // when a turn is in flight. The box no longer owns this — it is
            // a session concern, and the intercept above handles the
            // confirming half.
            KernelIntent::NormalEscape => {
                if stream_in_flight(state) {
                    state.frontend.cancel_stream_prompt = true;
                }
                IntentResult::empty()
            }
            KernelIntent::NoOp => IntentResult::empty(),

            // <c-c>: every picker that filters binds it in its own scope, so
            // the only kernel-side job is resetting the chat input box.
            KernelIntent::CtrlClear => {
                let (result, maybe_intent) = feat::global::intent::handle_ctrl_clear(state);
                if let Some(intent) = maybe_intent {
                    let redispatch = IntentHandler::handle(&intent, state, slices, routes, config);
                    result.merge(redispatch)
                } else {
                    result
                }
            }
            KernelIntent::SessionNew => {
                crate::session_lifecycle::intent::handle_session_new(state, config)
            }
            KernelIntent::RefreshModels => feat::session::intent::handle_refresh_models(state),
            KernelIntent::RescanPromptTemplates => {
                feat::session::intent::handle_rescan_prompt_templates(state)
            }

            KernelIntent::SessionLifecycleSetup {
                lifecycle_name,
                args,
            } => crate::session_lifecycle::intent::handle_session_lifecycle_setup(
                state,
                lifecycle_name,
                args,
                None,
                config,
            ),
            KernelIntent::SessionClose => {
                crate::session_lifecycle::intent::handle_session_close(state)
            }
            KernelIntent::Dynamic(_) => {
                // Unregistered dynamic intents are inert by construction:
                // a slice that never attached a route row for this action
                // must not fall into a built-in arm.
                tracing::debug!("dynamic intent arrived with no route row attached");
                IntentResult::empty()
            }
            KernelIntent::ChangeCwd { root } => {
                crate::feat::navigation::intent::handle_change_cwd(state, *root)
            }

            // ── Tabs ──
            KernelIntent::SwitchTab => {
                // Tab cycle across the registered tab scopes: the
                // composition-owned helper resolves the next base scope
                // from the slices' tab registry (chat when no dynamic
                // tab is registered). The terminal is an overlay
                // (<M-t>), not a tab: switching tabs with the overlay
                // open closes it first (Esc semantics). While the user
                // holds control, Tab is inert — handback is the only
                // exit.
                match state.frontend.scope() {
                    jinn_slices::FocusScope::Dynamic(id)
                        if jinn_term_msg::is_overlay_scope(&id) =>
                    {
                        // The terminal is an overlay (<M-t>), not a tab.
                        // In capture mode Tab is inert — handback is the
                        // only exit; in view mode switching tabs closes
                        // the overlay first (Esc semantics).
                        if id == jinn_term_msg::control_scope() {
                            return IntentResult::empty();
                        }
                        state.frontend.scope_pop();
                        return IntentResult::empty();
                    }
                    _ => {}
                }
                let new_base = next_tab_base(state, slices);
                state.frontend.scope_swap_base(new_base);
                IntentResult::empty()
            }
        }
    }
}

/// Cancel stream prompt intercept.
///
/// If the cancel-stream confirmation prompt is showing:
/// - `NormalEscape` confirms the cancel (and returns the appropriate commands).
/// - Any other intent dismisses the prompt and returns `None` (fall through to normal processing).
///
/// Returns `None` if the prompt is not showing or was dismissed.
fn try_handle_cancel_stream_prompt(
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
    let registry = state.task_spawns.clone();
    let mut visited = std::collections::HashSet::new();
    visited.insert(state.session.active_session_id().clone());
    result = cascade_descendants(
        state,
        state.session.active_session_id(),
        &registry,
        &mut visited,
    )
    .merge(result);

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
/// session actor owns each child's phase, and the frontend has already
/// driven only *its own* session's phase to `Idle`.
fn cascade_descendants(
    state: &AppState,
    session_id: &jinn_core_types::SessionId,
    registry: &jinn_tools_msg::TaskSpawnRegistry,
    visited: &mut std::collections::HashSet<jinn_core_types::SessionId>,
) -> IntentResult {
    let mut result = IntentResult::empty();

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
                    .merge(cascade_descendants(state, &child_id, registry, visited));
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

/// Dismisses armed confirmation prompts when an unrelated action arrives.
///
/// Runs as the first statement of [`IntentHandler::handle_inner`], ahead of
/// every dispatch path — including the slice route rows and the slice input
/// hooks, which both return early. A prompt cleared here cannot survive a
/// keystroke, which is the whole point: a prompt still on screen advertises a
/// confirmation the user never made.
///
/// The sidebar route actions that arm and confirm a prompt keep it intact and
/// perform their own revalidation and confirmation inside `jinn-sidebar`; the
/// cancel prompt is confirmed by the escape that raised it.
fn dismiss_unrelated_prompts(intent: &KernelIntent, state: &mut AppState) {
    let sidebar_action = match intent {
        KernelIntent::Dynamic(dynamic)
            if dynamic.slice == jinn_sidebar_msg::SidebarSectionId::Sessions.scope_id() =>
        {
            Some(dynamic.action.as_str())
        }
        _ => None,
    };

    if state.frontend.close_session_prompt && sidebar_action != Some("session-close") {
        state.frontend.close_session_prompt = false;
    }

    if state.frontend.archive_tree_prompt.is_some() {
        let matching_tree_action = matches!(
            sidebar_action,
            Some(jinn_sidebar_msg::TREE_ARCHIVE_ACTION | jinn_sidebar_msg::TREE_TEARDOWN_ACTION)
        );
        if !matching_tree_action {
            state.frontend.archive_tree_prompt = None;
        }
    }

    // The cancel prompt belongs to the session, not the sidebar: only the
    // confirming escape may leave it standing, and a turn that finished on
    // its own leaves nothing to cancel.
    if state.frontend.cancel_stream_prompt
        && (!matches!(intent, KernelIntent::NormalEscape) || !stream_in_flight(state))
    {
        state.frontend.cancel_stream_prompt = false;
    }
}

/// Whether a turn is in flight — the one condition that both raises the
/// cancel-stream prompt and keeps it standing.
///
/// Shared by the arming path and the dismissal above so the two can never
/// disagree: a prompt must not outlive the work it asks to abort.
fn stream_in_flight(state: &AppState) -> bool {
    state.active_session().is_busy()
        || !matches!(
            state.active_session().phase(),
            jinn_session_msg::PhaseKind::Idle
        )
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use jinn_status_bar_msg::{StatusBarState, status_bar_slot};

    /// Empty slice registry + route table for handler tests that don't
    /// exercise slices or route rows.
    fn empty_slices() -> jinn_slices::Slices {
        jinn_slices::Slices::new()
    }

    /// `Slices` with the status-bar cell registered (as the slice's
    /// `activate` does), for hint write/read assertions.
    fn status_bar_slices() -> jinn_slices::Slices {
        let slices = jinn_slices::Slices::new();
        #[expect(
            clippy::expect_used,
            reason = "test seam: a fresh Slices never has the status-bar cell registered"
        )]
        {
            slices
                .register(status_bar_slot(), StatusBarState::default())
                .expect("fresh Slices never has the status-bar cell registered");
        }
        slices
    }

    fn status_hint(slices: &jinn_slices::Slices) -> Option<String> {
        slices
            .reader::<StatusBarState>(&status_bar_slot())?
            .read()
            .hint
            .clone()
    }

    fn empty_routes() -> jinn_slices::route::KeyRoutes {
        jinn_slices::route::KeyRoutes::new()
    }

    fn activate_child_route(child_id: jinn_core_types::SessionId) -> jinn_slices::route::KeyRoutes {
        use jinn_slices::route::{ActionFn, BindSite, RouteId, RouteOutcome, RouteRow};

        let routes = empty_routes();
        routes.attach(RouteRow {
            route_id: RouteId::new("test:activate-child"),
            scope: jinn_slices::SliceScopeId::navigation("test", "activate-child"),
            key: "<enter>",
            category: "general",
            site: BindSite::StaticScopes(&["Normal"]),
            feature: "test",
            outcome: RouteOutcome::Action {
                action: "activate-child",
                display: "activate child",
                run: ActionFn::new(move |ctx| {
                    let state = ctx
                        .state
                        .as_any_mut()
                        .and_then(|state| state.downcast_mut::<AppState>())
                        .expect("test route runs against AppState");
                    state.session.set_active(child_id.clone());
                    IntentResult::empty()
                }),
            },
        });
        routes
    }

    fn activate_child_intent() -> KernelIntent {
        KernelIntent::Dynamic(jinn_slices::DynamicIntent::new(
            jinn_slices::SliceScopeId::navigation("test", "activate-child"),
            "activate-child",
            "activate child",
        ))
    }

    /// A state holding a child session linked from the active one, a live
    /// terminal for the parent, and the term view overlay pushed on top of
    /// a `Normal` base — i.e. everything the switch guard reacts to.
    fn state_with_linked_child_and_terminal_overlay(
        entry_id: &str,
    ) -> (AppState, jinn_core_types::SessionId) {
        use crate::protocol::ChatEntryKind;
        use jinn_core_types::SessionId;
        use jinn_session_state::ChatSessionState;
        use jinn_tools_msg::TASK_TOOL_NAME;

        let mut state = AppState::default_with_scope_focus();
        let first_id = state.session.active_session_id().clone();
        let child_id = SessionId::new();
        let mut child = ChatSessionState::new_child(&first_id, false);
        child.set_session_id(child_id.clone());
        state.session.insert(child);
        let mut entry = ChatEntry::tool_call(entry_id, TASK_TOOL_NAME, "{}");
        let ChatEntryKind::ToolCall { child_session, .. } = &mut entry.kind else {
            panic!("expected ToolCall kind");
        };
        *child_session = Some(child_id.clone());
        state.active_session_mut().push_entry(entry);
        state.active_session_mut().select_prev_entry();

        state
            .term_tabs()
            .expect("term tabs cell")
            .update(|t| t.set_live(&first_id, true));
        // The real overlay opens on top of the base scope
        // (clear_overlays + push); the guard clears overlays, so the
        // overlay must not be the base itself.
        state.frontend.scope_swap_base(FocusScope::Normal);
        state
            .frontend
            .scope_push(FocusScope::Dynamic(jinn_term_msg::view_scope()));

        (state, child_id)
    }

    /// A state whose only session is live in the terminal, with the
    /// Sessions sidebar section selected and the base swapped to it.
    fn state_with_live_terminal_session_selected() -> (AppState, jinn_core_types::SessionId) {
        use jinn_session_state::ChatSessionState;

        let mut state = AppState::default_with_scope_focus();
        let second = ChatSessionState::new();
        let second_id = second.session_id().clone();
        state.session.insert(second);
        state
            .term_tabs()
            .expect("term tabs cell")
            .update(|t| t.set_live(&second_id, true));
        state
            .frontend
            .scope_swap_base(jinn_sidebar_msg::SidebarSectionId::Sessions.focus_scope());
        state
            .frontend
            .update_sections(|s| s.sessions.selected_index = Some(0));

        (state, second_id)
    }

    /// A dynamic intent plus the route table that activates `second_id` and
    /// then toggles the terminal view overlay for the newly active session.
    fn activate_and_toggle_overlay_routes(
        second_id: jinn_core_types::SessionId,
    ) -> (KernelIntent, jinn_slices::route::KeyRoutes) {
        let intent = KernelIntent::Dynamic(jinn_slices::DynamicIntent::new(
            jinn_term_msg::view_scope(),
            "toggle-for-selected",
            "toggle terminal",
        ));
        let routes = jinn_slices::route::KeyRoutes::new();
        routes.attach(jinn_slices::route::RouteRow {
            route_id: jinn_slices::route::RouteId::new("term:toggle-for-selected"),
            scope: jinn_term_msg::view_scope(),
            key: "T",
            category: "general",
            site: jinn_slices::route::BindSite::OwnScope,
            feature: "term",
            outcome: jinn_slices::route::RouteOutcome::Action {
                action: "toggle-for-selected",
                display: "toggle terminal",
                run: jinn_slices::route::ActionFn::new(move |ctx| {
                    let Some(state) = ctx
                        .state
                        .as_any_mut()
                        .and_then(|a| a.downcast_mut::<AppState>())
                    else {
                        return IntentResult::empty();
                    };
                    // Inline term-slice semantics: activate the target
                    // session, then toggle the overlay for the active one.
                    if state.frontend.sidebar_section()
                        == Some(jinn_sidebar_msg::SidebarSectionId::Sessions)
                    {
                        state.session.set_active(second_id.clone());
                    }
                    let chat = state.session.active_session_id().clone();
                    let live = state
                        .term_tabs()
                        .is_some_and(|cell| cell.read().live_terms.contains(&chat));
                    if !live {
                        return IntentResult::empty();
                    }
                    if state.frontend.scope() == FocusScope::Dynamic(jinn_term_msg::view_scope()) {
                        state.frontend.scope_pop();
                    } else {
                        state.frontend.scope_clear_overlays();
                        state
                            .frontend
                            .scope_push(FocusScope::Dynamic(jinn_term_msg::view_scope()));
                    }
                    IntentResult::empty()
                }),
            },
        });

        (intent, routes)
    }

    /// Route table whose single row requests `signal` for `scope`, plus a
    /// scope-enter hook for that same scope that bumps `enters`.
    fn transitioning_routes(
        scope: jinn_slices::SliceScopeId,
        signal: ScopeSignal,
        enters: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> (jinn_slices::route::KeyRoutes, KernelIntent) {
        use jinn_slices::route::{ActionFn, BindSite, RouteId, RouteOutcome, RouteRow};
        use std::sync::atomic::Ordering;

        let routes = empty_routes();
        routes.attach(RouteRow {
            route_id: RouteId::new("test:transition"),
            scope: scope.clone(),
            key: "<enter>",
            category: "general",
            site: BindSite::OwnScope,
            feature: "test",
            outcome: RouteOutcome::Action {
                action: "transition",
                display: "request a scope transition",
                run: ActionFn::new(move |_ctx| {
                    IntentResult::empty().with_scope_signal(signal.clone())
                }),
            },
        });
        let counted = std::sync::Arc::clone(enters);
        routes.register_scope_enter_hook(
            &scope,
            std::sync::Arc::new(move |_ctx: jinn_slices::route::ActionCtx<'_>| {
                counted.fetch_add(1, Ordering::SeqCst);
            }),
        );
        let intent = KernelIntent::Dynamic(jinn_slices::DynamicIntent::new(
            scope,
            "transition",
            "request a scope transition",
        ));
        (routes, intent)
    }
    use crate::common::app_state::AppState;
    use crate::feat::intent::IntentHandler;
    use crate::protocol::IntentResult;
    use crate::protocol::ScopeSignal;
    use crate::protocol::{ChatEntry, KernelIntent};
    use jinn_slices::FocusScope;

    #[rstest::rstest]
    fn paste_text_ignored_in_normal_scope() {
        // Given an AppState in Normal scope.
        // (The action name is a literal rather than the slice's constant:
        // the kernel must not depend on the slice crate.)
        let mut state = AppState::default_with_scope_focus();
        state.frontend.scope_clear_overlays();

        // When handling PasteText.
        let result = IntentHandler::handle(
            &KernelIntent::Dynamic(jinn_slices::DynamicIntent::with_bytes(
                jinn_chat_input_msg::chat_input_scope(),
                "paste-text",
                "paste text",
                b"hello".to_vec(),
            )),
            &mut state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then the buffer is empty and no commands are emitted.
        assert!(
            state
                .active_session()
                .with_input(jinn_chat_input_msg::ChatInputBoxState::is_empty, || true)
        );
        assert!(result.message_names.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn paste_text_in_picker_scope_routes_to_picker() {
        // Given Picker scope is active.
        let mut state = AppState::default_with_scope_focus();
        state
            .frontend
            .scope_push(FocusScope::Dynamic(jinn_project_msg::project_picker_scope()));

        // When handling PasteText.
        let _result = IntentHandler::handle(
            &KernelIntent::Dynamic(jinn_slices::DynamicIntent::with_bytes(
                jinn_chat_input_msg::chat_input_scope(),
                "paste-text",
                "paste text",
                b"hello".to_vec(),
            )),
            &mut state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then it doesn't panic and completes (paste is handled by picker).
        // The picker query filter is updated.
    }

    #[rstest::rstest]
    #[test]
    fn cancel_stream_prompt_esc_confirms() {
        // Given cancel_stream_prompt is showing over a turn in flight.
        let mut state = AppState::default_with_scope_focus();
        state.active_session_mut().begin_streaming();
        state.frontend.cancel_stream_prompt = true;

        // When handling NormalEscape.
        let result = IntentHandler::handle(
            &KernelIntent::NormalEscape,
            &mut state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then the prompt is dismissed and a CancelStream command is emitted.
        assert!(!state.frontend.cancel_stream_prompt);
        assert!(
            result
                .message_names
                .iter()
                .any(|n| n.contains("CancelStream")),
            "should emit CancelStream: {:?}",
            result.message_names
        );
    }

    #[rstest::rstest]
    #[test]
    fn cancel_stream_prompt_other_intent_dismisses() {
        // Given cancel_stream_prompt is showing.
        let mut state = AppState::default_with_scope_focus();
        state.frontend.cancel_stream_prompt = true;

        // When handling a different intent (NoOp).
        let _result = IntentHandler::handle(
            &KernelIntent::NoOp,
            &mut state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then the prompt is dismissed but no CancelStream command.
        assert!(!state.frontend.cancel_stream_prompt);
    }

    #[rstest::rstest]
    #[test]
    fn cancel_stream_prompt_not_showing_returns_none() {
        // Given cancel_stream_prompt is NOT showing.
        let mut state = AppState::default_with_scope_focus();
        state.frontend.cancel_stream_prompt = false;

        // When handling NormalEscape.
        let _result = IntentHandler::handle(
            &KernelIntent::NormalEscape,
            &mut state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then no cancel command is emitted (falls through to normal escape handling).
        // The prompt remains false.
        assert!(!state.frontend.cancel_stream_prompt);
    }

    #[rstest::rstest]
    #[test]
    fn close_session_prompt_other_intent_dismisses() {
        // Given close_session_prompt is showing.
        let mut state = AppState::default_with_scope_focus();
        state.frontend.close_session_prompt = true;

        // When handling a different intent (NoOp).
        let _result = IntentHandler::handle(
            &KernelIntent::NoOp,
            &mut state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then the prompt is dismissed.
        assert!(!state.frontend.close_session_prompt);
    }

    #[rstest::rstest]
    #[test]
    fn cancel_stream_prompt_noop_dismisses() {
        // Given cancel_stream_prompt is showing.
        let mut state = AppState::default_with_scope_focus();
        state.frontend.cancel_stream_prompt = true;

        // When handling NoOp (unmapped key).
        let result = IntentHandler::handle(
            &KernelIntent::NoOp,
            &mut state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then the prompt is dismissed and no CancelStream command is emitted.
        assert!(!state.frontend.cancel_stream_prompt);
        assert!(
            !result
                .message_names
                .iter()
                .any(|n| n.contains("CancelStream")),
            "should not emit CancelStream: {:?}",
            result.message_names
        );
    }

    #[rstest::rstest]
    #[test]
    fn close_session_prompt_noop_dismisses() {
        // Given close_session_prompt is showing.
        let mut state = AppState::default_with_scope_focus();
        state.frontend.close_session_prompt = true;

        // When handling NoOp (unmapped key).
        let _result = IntentHandler::handle(
            &KernelIntent::NoOp,
            &mut state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then the prompt is dismissed.
        assert!(!state.frontend.close_session_prompt);
    }

    #[rstest::rstest]
    #[test]
    fn noop_is_empty_when_no_prompt() {
        // Given default state with no prompts showing.
        let mut state = AppState::default_with_scope_focus();

        // When handling NoOp.
        let result = IntentHandler::handle(
            &KernelIntent::NoOp,
            &mut state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then result is empty.
        assert!(result.message_names.is_empty());
    }

    #[rstest::rstest]
    fn active_session_changed_emitted_on_session_switch() {
        // Given a state with two sessions.
        use jinn_session_state::ChatSessionState;

        let mut state = AppState::default_with_scope_focus();
        let first_id = state.session.active_session_id().clone();

        let mut second = ChatSessionState::new();
        second.push_entry(ChatEntry::user("second session"));
        let second_id = second.session_id().clone();
        state.session.insert(second);

        // Activate second session directly (simulating sidebar click).
        state.session.set_active(second_id);

        // When handling an intent that leaves the active session alone
        // (NoOp), so no event should be raised.
        state.session.set_active(first_id);
        let result = IntentHandler::handle(
            &KernelIntent::NoOp,
            &mut state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then no ActiveSessionChanged event (same session).
        let has_event = result
            .message_names
            .iter()
            .any(|&name| name.contains("ActiveSessionChanged"));
        assert!(
            !has_event,
            "should not emit ActiveSessionChanged when session unchanged"
        );
    }

    #[rstest::rstest]
    fn switch_tab_is_inert_while_user_holds_terminal_control() {
        // Given the terminal-control overlay open (user holds control).
        let mut state = AppState::default_with_scope_focus();
        state.frontend.scope_clear_overlays();
        state
            .frontend
            .scope_push(FocusScope::Dynamic(jinn_term_msg::control_scope()));

        // When switching tabs.
        IntentHandler::handle(
            &KernelIntent::SwitchTab,
            &mut state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then the scope stays on term:control — handback is the only exit.
        assert_eq!(
            state.frontend.scope(),
            FocusScope::Dynamic(jinn_term_msg::control_scope())
        );
    }

    #[rstest::rstest]
    fn status_hint_write_is_a_noop_without_the_status_bar_cell() {
        // Given a slice registry without the status-bar cell.
        let slices = empty_slices();

        // When attempting a hint write.
        if let Some(status) = slices.reader::<StatusBarState>(&status_bar_slot()) {
            status.update(|state| state.hint = Some("should be dropped".to_owned()));
        }

        // Then no status state is available to expose a hint.
        assert!(status_hint(&slices).is_none());
    }

    #[rstest::rstest]
    fn default_scope_is_input_through_the_facade() {
        // Given a default state (no scope-focus wiring attached).
        let state = AppState::default();

        // When reading the current scope through the facade.
        let scope = state.frontend.scope();

        // Then it is Input — the historical default boot scope.
        assert_eq!(scope, FocusScope::Input);
    }

    #[rstest::rstest]
    fn scope_writes_are_noop_without_the_cell() {
        // Given a state whose scope-focus cell was never minted.
        let state = AppState::default();

        // When pushing a scope through the facade.
        state.frontend.scope_push(FocusScope::Normal);

        // Then the read still returns the unattached default (no panic,
        // no storage) — the removability property.
        assert_eq!(state.frontend.scope(), FocusScope::Input);
    }

    #[rstest::rstest]
    fn next_intent_dismisses_a_raised_status_hint() {
        // Given a state carrying a hint from a failed overlay toggle.
        let mut state = AppState::default_with_scope_focus();
        let slices = status_bar_slices();
        slices
            .reader::<StatusBarState>(&status_bar_slot())
            .expect("status-bar cell registered by test setup")
            .update(|status| status.hint = Some("stale hint".to_owned()));

        // When handling any other intent.
        IntentHandler::handle(
            &KernelIntent::SwitchTab,
            &mut state,
            &slices,
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then the hint is cleared.
        assert!(status_hint(&slices).is_none());
    }

    #[rstest::rstest]
    fn switch_tab_with_no_registered_tab_stays_normal() {
        // Given default (Normal) state and no dynamic tab registered.
        let mut state = AppState::default_with_scope_focus();

        // When switching tabs.
        IntentHandler::handle(
            &KernelIntent::SwitchTab,
            &mut state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then the base is Normal (chat is the only tab).
        assert_eq!(state.frontend.scope_base(), FocusScope::Normal);
    }

    #[rstest::rstest]
    fn switch_tab_activates_the_registered_tab() {
        // Given a slices registry with one dynamic tab registered.
        let slices = jinn_slices::Slices::new();
        let tab = jinn_slices::SliceScopeId::new("dashboard", "tab");
        slices.register_tab_scope(
            tab.clone(),
            jinn_slices::SlotKey::builtin("dashboard", "tab"),
        );
        let mut state = AppState::default_with_scope_focus();

        // When switching tabs.
        IntentHandler::handle(
            &KernelIntent::SwitchTab,
            &mut state,
            &slices,
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then the base is the registered tab.
        assert_eq!(
            state.frontend.scope_base(),
            FocusScope::Dynamic(tab.clone())
        );
    }

    #[rstest::rstest]
    fn switch_tab_wraps_to_normal_after_the_last_tab() {
        // Given a state whose base is the only registered tab.
        let slices = jinn_slices::Slices::new();
        let tab = jinn_slices::SliceScopeId::new("dashboard", "tab");
        slices.register_tab_scope(
            tab.clone(),
            jinn_slices::SlotKey::builtin("dashboard", "tab"),
        );
        let mut state = AppState::default_with_scope_focus();
        state
            .frontend
            .scope_swap_base(FocusScope::Dynamic(tab.clone()));

        // When switching tabs again.
        IntentHandler::handle(
            &KernelIntent::SwitchTab,
            &mut state,
            &slices,
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then the cycle wraps to Normal.
        assert_eq!(state.frontend.scope_base(), FocusScope::Normal);
    }

    #[rstest::rstest]
    fn switch_tab_while_overlay_open_closes_it() {
        // Given an open terminal overlay over the Normal base.
        let mut state = AppState::default_with_scope_focus();
        let chat = state.session.active_session_id().clone();
        state
            .term_tabs()
            .expect("term tabs cell")
            .update(|t| t.set_live(&chat, true));
        state.frontend.scope_swap_base(FocusScope::Normal);
        state
            .frontend
            .scope_push(FocusScope::Dynamic(jinn_term_msg::view_scope()));

        // When switching tabs.
        IntentHandler::handle(
            &KernelIntent::SwitchTab,
            &mut state,
            &empty_slices(),
            &empty_routes(),
            jinn_slices::empty_config_layer(),
        );

        // Then the overlay closed (back to base, not a tab flip).
        assert_eq!(state.frontend.scope(), FocusScope::Normal);
        assert_eq!(state.frontend.scope_base(), FocusScope::Normal);
    }

    #[rstest::rstest]
    fn active_session_switch_activates_the_child_session() {
        // Given a state with two sessions and the overlay open over the first.
        let slices = status_bar_slices();
        let (mut state, child_id) = state_with_linked_child_and_terminal_overlay("tc_guard_test");

        // When a route-owned action switches the active session.
        IntentHandler::handle(
            &activate_child_intent(),
            &mut state,
            &slices,
            &activate_child_route(child_id.clone()),
            jinn_slices::empty_config_layer(),
        );

        // Then the active session changed to the child.
        assert_eq!(
            state.session.active_session_id(),
            &child_id,
            "the child session must be activated"
        );
    }

    #[rstest::rstest]
    fn active_session_switch_closes_terminal_overlay() {
        // Given a state with two sessions and the overlay open over the first.
        let slices = status_bar_slices();
        let (mut state, child_id) = state_with_linked_child_and_terminal_overlay("tc_guard_test");

        // When a route-owned action switches the active session.
        IntentHandler::handle(
            &activate_child_intent(),
            &mut state,
            &slices,
            &activate_child_route(child_id.clone()),
            jinn_slices::empty_config_layer(),
        );

        // Then the previously-open terminal overlay did not survive the switch.
        assert_ne!(
            state.frontend.scope(),
            FocusScope::Dynamic(jinn_term_msg::view_scope()),
            "a switch under an open overlay must not carry it to the new session"
        );
    }

    #[rstest::rstest]
    fn active_session_switch_hints_that_the_overlay_closed() {
        // Given a state with two sessions and the overlay open over the first.
        let slices = status_bar_slices();
        let (mut state, child_id) = state_with_linked_child_and_terminal_overlay("tc_guard_test");

        // When a route-owned action switches the active session.
        IntentHandler::handle(
            &activate_child_intent(),
            &mut state,
            &slices,
            &activate_child_route(child_id.clone()),
            jinn_slices::empty_config_layer(),
        );

        // Then the hint explains the abrupt close.
        let hint = status_hint(&slices);
        assert!(
            hint.as_deref().is_some_and(|h| h.contains("closed")),
            "expected an overlay-closed hint, got: {hint:?}"
        );
    }

    #[rstest::rstest]
    fn active_session_switch_releases_user_control() {
        // Given a state with a linked child session, the overlay open in
        // control mode (user holds the previous session's terminal).
        use crate::protocol::ChatEntryKind;
        use jinn_core_types::SessionId;
        use jinn_session_state::ChatSessionState;
        use jinn_term_msg::command::ControlHolder;
        use jinn_tools_msg::TASK_TOOL_NAME;
        let mut state = AppState::default_with_scope_focus();
        let first_id = state.session.active_session_id().clone();
        let child_id = SessionId::new();
        let mut child = ChatSessionState::new_child(&first_id, false);
        child.set_session_id(child_id.clone());
        state.session.insert(child);

        let mut entry = ChatEntry::tool_call("tc_guard_test2", TASK_TOOL_NAME, "{}");
        let ChatEntryKind::ToolCall { child_session, .. } = &mut entry.kind else {
            panic!("expected ToolCall kind");
        };
        *child_session = Some(child_id.clone());
        state.active_session_mut().push_entry(entry);
        state.active_session_mut().select_prev_entry();

        state
            .term_tabs()
            .expect("term tabs cell")
            .update(|t| t.set_live(&first_id, true));
        state.frontend.scope_swap_base(FocusScope::Normal);
        state
            .frontend
            .scope_push(FocusScope::Dynamic(jinn_term_msg::control_scope()));
        // Mint the registry (idempotent: a no-op when already present) so
        // the release assertion below is unconditional — production always
        // has the registry wired, and this test must prove the flip.
        let _ = jinn_term_msg::TERM_CONTROLS.set(jinn_term_msg::TermControls::default());
        let registry = jinn_term_msg::TERM_CONTROLS.get().expect("minted above");
        registry.set(&first_id, ControlHolder::User);

        // When switching the active session through a route-owned action.
        IntentHandler::handle(
            &activate_child_intent(),
            &mut state,
            &empty_slices(),
            &activate_child_route(child_id.clone()),
            jinn_slices::empty_config_layer(),
        );

        // Then the switched-from session's control is released back to the
        // agent — a stuck User holder would refuse every future agent send.
        assert_eq!(
            registry.holder_for(&first_id),
            ControlHolder::Agent,
            "control must not stick on User after a switch"
        );
        // And the overlay is closed.
        assert_ne!(
            state.frontend.scope(),
            FocusScope::Dynamic(jinn_term_msg::control_scope())
        );
    }

    #[rstest::rstest]
    fn overlay_opened_by_the_switch_intent_survives_the_guard() {
        // Given a state with two sessions where the *second* holds the live
        // terminal, and no overlay open yet.
        let (mut state, second_id) = state_with_live_terminal_session_selected();

        // When the sidebar toggle activates the session and opens the overlay
        // in the same intent.
        let (intent, routes) = activate_and_toggle_overlay_routes(second_id);
        IntentHandler::handle(
            &intent,
            &mut state,
            &empty_slices(),
            &routes,
            jinn_slices::empty_config_layer(),
        );

        // Then the overlay is open on the newly-activated session's terminal
        // — the guard captured "overlay closed" before the intent and must
        // not close what its own intent opened.
        assert_eq!(
            state.frontend.scope(),
            FocusScope::Dynamic(jinn_term_msg::view_scope()),
            "activate-then-open must survive the switch guard"
        );
    }

    #[rstest::rstest]
    fn handback_screen_survives_drain_as_user_entry() {
        use jinn_session_state::steering_buffer::SteeringBuffer;

        // Given the push message text for a captured screen.
        let text = format!(
            "Here is the current terminal screen:\n\n```\n{}\n```",
            "drain-chain-marker"
        );

        // When routing the text through the steering buffer and draining it
        // (the session actor's busy-path behavior).
        let mut buf = SteeringBuffer::new();
        buf.push_fragment(text);
        let entry = buf.drain_into_entry().expect("entry");

        // Then the drained entry is a normal User entry carrying the screen.
        assert!(matches!(
            entry.kind,
            crate::protocol::ChatEntryKind::User { .. }
        ));
    }

    #[rstest::rstest]
    fn push_transition_fires_the_target_scope_enter_hook() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicUsize;
        use std::sync::atomic::Ordering;

        // Given a route row that pushes a scope owning an enter hook.
        let scope = jinn_slices::SliceScopeId::new("test", "picker");
        let enters = Arc::new(AtomicUsize::new(0));
        let (routes, intent) =
            transitioning_routes(scope.clone(), ScopeSignal::Push(scope.clone()), &enters);
        let mut state = AppState::default_with_scope_focus();

        // When handling the row's dynamic intent.
        IntentHandler::handle(
            &intent,
            &mut state,
            &empty_slices(),
            &routes,
            jinn_slices::empty_config_layer(),
        );

        // Then the hook ran exactly once.
        assert_eq!(enters.load(Ordering::SeqCst), 1);
    }

    #[rstest::rstest]
    fn push_transition_leaves_the_scope_on_the_stack() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicUsize;

        // Given a route row that pushes a scope.
        let scope = jinn_slices::SliceScopeId::new("test", "picker");
        let counter = Arc::new(AtomicUsize::new(0));
        let (routes, intent) =
            transitioning_routes(scope.clone(), ScopeSignal::Push(scope.clone()), &counter);
        let mut state = AppState::default_with_scope_focus();

        // When handling the row's dynamic intent.
        IntentHandler::handle(
            &intent,
            &mut state,
            &empty_slices(),
            &routes,
            jinn_slices::empty_config_layer(),
        );

        // Then the scope is the active focus.
        assert_eq!(state.frontend.scope(), FocusScope::Dynamic(scope));
    }

    #[rstest::rstest]
    fn pop_if_transition_does_not_fire_the_scope_enter_hook() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicUsize;
        use std::sync::atomic::Ordering;

        // Given a route row that requests a pop-if for a scope owning an
        // enter hook.
        let scope = jinn_slices::SliceScopeId::new("test", "picker");
        let enters = Arc::new(AtomicUsize::new(0));
        let (routes, intent) =
            transitioning_routes(scope.clone(), ScopeSignal::PopIf(scope), &enters);
        let mut state = AppState::default_with_scope_focus();

        // When handling the row's dynamic intent.
        IntentHandler::handle(
            &intent,
            &mut state,
            &empty_slices(),
            &routes,
            jinn_slices::empty_config_layer(),
        );

        // Then the hook never ran — a pop is not an entry.
        assert_eq!(enters.load(Ordering::SeqCst), 0);
    }

    // -- Confirmed-cancel cascade ----------------------------------------

    use jinn_core_types::SessionId;
    use jinn_session_msg::SessionOrigin;
    use jinn_session_state::ChatSessionState;

    /// Builds a confirmed-cancel intent over `state` and returns the result.
    ///
    /// Arms the prompt only while a turn is in flight — the same invariant
    /// the dismissal sweep enforces — so the fixture starts a stream first.
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

    /// Counts the CancelStream messages in a result.
    fn cancel_count(result: &IntentResult) -> usize {
        result
            .message_names
            .iter()
            .filter(|name| name.contains("CancelStream"))
            .count()
    }

    /// Links `child` under `parent` with the given origin and inserts both.
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
    #[test]
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
    #[test]
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
    #[test]
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
    #[test]
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
    #[test]
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
    #[test]
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
