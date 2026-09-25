//! Spec-driven picker actions — the registry-first dispatch layer.
//!
//! These functions sit between the intent handler and the erased specs:
//! they resolve the active picker's spec through the [`PickerRegistry`],
//! build the [`ActionCtx`] over the domain [`PickerHost`] impl, run the
//! hook, and fold the [`PickerOutcome`] into an [`IntentResult`] (messages
//! drained; optional picker closure precedes the requested scope transition).
//!
//! Every function is a no-op returning an empty result when the active
//! picker has no spec (a kind with no registered spec) or the id/action
//! doesn't resolve — stale intents are ignored, never panics.

use jinn_picker::ActionCtx;
use jinn_picker::PickerOutcome;
use jinn_picker::PickerRegistry;

use crate::common::app_state::AppState;
use crate::common::slices::key_routes::apply_scope_signal;
use crate::feat::picker::host_impl::AppStatePickerHost;
use crate::protocol::intent::IntentResult;

/// Folds a picker outcome into an intent result.
///
/// When requested, `close` clears overlay scopes back to the current base
/// before `scope_signal` is applied. A picker opened from Input (for
/// example, the model picker) therefore cannot strand the user in a stale
/// Input scope, while a destination pushed by the outcome survives closure.
fn fold(state: &mut AppState, outcome: PickerOutcome) -> IntentResult {
    let mut result = IntentResult {
        messages: outcome.messages,
        message_names: outcome.message_names,
        scope_signal: outcome.scope_signal,
    };
    if outcome.close {
        state.frontend.scope_clear_overlays();
    }
    apply_scope_signal(&mut result, state);
    result
}

/// Runs the active picker's spec hook for `which`, when declared.
///
/// Per-hook fallback: a lifecycle step is spec-driven only when the
/// spec *declares that hook* — a step the spec hasn't taken over falls
/// through to the per-kind handlers.
pub fn run_active_hook(
    state: &mut AppState,
    registry: &PickerRegistry,
    hook: Hook,
) -> IntentResult {
    let Some(kind) = state.frontend.picker_kind() else {
        return IntentResult::empty();
    };
    let Some(id) = jinn_picker::spec_id_for_kind(&kind) else {
        return IntentResult::empty();
    };
    let Some(spec) = registry.get(id) else {
        return IntentResult::empty();
    };
    let declared = match hook {
        Hook::Open => spec.has_open(),
        Hook::Confirm => spec.has_confirm(),
        Hook::Close => spec.has_close(),
    };
    if !declared {
        return IntentResult::empty();
    }
    let picker_id = jinn_picker::PickerId::new(spec.id().as_str());
    let outcome = {
        let mut host = AppStatePickerHost::new(state);
        let mut ctx = ActionCtx::new(picker_id, &mut host);
        match hook {
            Hook::Open => spec.run_open(&mut ctx),
            Hook::Confirm => spec.run_confirm(&mut ctx),
            Hook::Close => spec.run_close(&mut ctx),
        }
    };
    fold(state, outcome)
}

/// Runs the active picker's selection-change hook when its spec declares
/// one — the live preview fired after the cursor moved or the page turned.
/// A no-op when no spec is active or the spec has no selection-change
/// behavior.
pub fn run_selection_change(state: &mut AppState, registry: &PickerRegistry) {
    let Some(kind) = state.frontend.picker_kind() else {
        return;
    };
    let Some(id) = jinn_picker::spec_id_for_kind(&kind) else {
        return;
    };
    let Some(spec) = registry.get(id) else {
        return;
    };
    if !spec.has_selection_change() {
        return;
    }
    // The cursor position lives in the picker's selection storage, whose
    // concrete type only the typed spec knows — resolve it through the
    // erased seam, then run the hook.
    let picker_id = jinn_picker::PickerId::new(spec.id().as_str());
    let index = {
        let host = crate::feat::picker::host_impl::AppStateRenderHost::new(state);
        spec.selected_index(&host)
    };
    let mut host = AppStatePickerHost::new(state);
    let mut ctx = ActionCtx::new(picker_id, &mut host);
    spec.run_selection_change(index, &mut ctx);
}

/// The lifecycle hook to run for the active picker.
#[derive(Debug, Clone, Copy)]
pub enum Hook {
    /// The open hook.
    Open,
    /// The confirm hook (Enter).
    Confirm,
    /// The close hook (ESC revert path).
    Close,
}

/// Runs the active picker's close hook when it has a spec. Returns
/// `Some(result)` when the hook ran (the caller stops — a per-kind restore
/// must not double-apply), `None` when no spec is active.
pub fn try_close_active(state: &mut AppState, registry: &PickerRegistry) -> Option<IntentResult> {
    let kind = state.frontend.picker_kind()?;
    let id = jinn_picker::spec_id_for_kind(&kind)?;
    let spec = registry.get(id)?;
    if !spec.has_close() {
        return None;
    }
    Some(run_active_hook(state, registry, Hook::Close))
}

/// Runs the `picker` spec's `action`-named bind.
pub fn run_action(
    state: &mut AppState,
    registry: &PickerRegistry,
    picker: &str,
    action: &str,
) -> IntentResult {
    // Guard: the intent's picker must be the one actually open — a stale
    // keypress from a previous scope is ignored.
    let Some(kind) = state.frontend.picker_kind() else {
        return IntentResult::empty();
    };
    let Some(active_id) = jinn_picker::spec_id_for_kind(&kind) else {
        return IntentResult::empty();
    };
    if active_id != picker {
        return IntentResult::empty();
    }
    let Some(spec) = registry.get(picker) else {
        return IntentResult::empty();
    };
    let picker_id = jinn_picker::PickerId::new(spec.id().as_str());
    let outcome = {
        let mut host = AppStatePickerHost::new(state);
        let mut ctx = ActionCtx::new(picker_id, &mut host);
        spec.run_action(action, &mut ctx)
    };
    fold(state, outcome)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test module, panics are acceptable"
    )]
    use super::*;
    use crate::protocol::ChatEntryKind;
    use jinn_picker::PERSONA_ID;
    use jinn_slices::FocusScope;
    use jinn_slices::ScopeSignal;
    use jinn_slices::SliceScopeId;

    fn state_with_persona_picker() -> AppState {
        let state = AppState::default_with_scope_focus();
        state.frontend.scope_push(FocusScope::Picker {
            kind: crate::PickerKind::Persona,
        });
        state
    }

    /// A test spec under the persona id with one `<tab>` bind that pushes a
    /// transient entry, and one `<esc>` bind that closes the picker.
    ///
    /// Built here rather than imported from `jinn_picker_specs`: these tests
    /// exercise dispatch, and the kernel cannot depend on the specs crate.
    fn registry_with_test_persona_spec() -> jinn_picker::PickerRegistry {
        let spec = jinn_picker::PickerSpec::<super::super::test_registry::Entry>::new(
            jinn_picker::PickerId::new(PERSONA_ID),
        )
        .bind("<tab>", "test", |ctx: &mut ActionCtx<'_>| {
            let state = ctx
                .state_any()
                .downcast_mut::<AppState>()
                .expect("domain host lends AppState");
            state
                .active_session_mut()
                .push_entry(crate::protocol::ChatEntry::transient("test bind ran"));
            jinn_picker::PickerOutcome::empty()
        })
        .bind("<esc>", "close", |_ctx: &mut ActionCtx<'_>| {
            jinn_picker::PickerOutcome::empty().close()
        });
        let mut registry = jinn_picker::PickerRegistry::new();
        registry.register(spec);
        registry
    }

    #[rstest::rstest]
    #[test]
    fn picker_action_unknown_id_is_a_no_op() {
        // Given an open persona picker and the domain registry.
        let mut state = state_with_persona_picker();
        let registry = crate::feat::picker::test_registry::test_registry();

        // When running an action naming a picker id that doesn't exist.
        let result = run_action(&mut state, &registry, "nope", "<tab>");

        // Then nothing is emitted.
        assert!(result.messages.is_empty());
        assert!(result.message_names.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn picker_action_with_wrong_active_picker_is_ignored() {
        // Given an open persona picker.
        let mut state = state_with_persona_picker();
        let registry = crate::feat::picker::test_registry::test_registry();

        // When running an action addressed to a different picker.
        let result = run_action(&mut state, &registry, "theme", "<tab>");

        // Then nothing is emitted (stale intents are dropped).
        assert!(result.messages.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn picker_action_resolves_the_row_and_runs_it() {
        // Given an open persona picker whose test spec declares a `<tab>` row
        // that pushes a transient entry.
        let mut state = state_with_persona_picker();
        let registry = registry_with_test_persona_spec();

        // When running the `<tab>` action.
        let _ = run_action(&mut state, &registry, PERSONA_ID, "<tab>");

        // Then the action ran (transient entry pushed).
        let history = state.active_session().history();
        assert!(
            matches!(
                history.last(),
                Some(entry) if matches!(entry.kind, ChatEntryKind::Transient(_))
            ),
            "the <tab> bind action should have run"
        );
    }

    #[rstest::rstest]
    #[test]
    fn picker_action_close_outcome_pops_the_scope() {
        // Given an open persona picker.
        let mut state = state_with_persona_picker();
        let registry = registry_with_test_persona_spec();

        // When running an action whose outcome closes the picker.
        let _ = run_action(&mut state, &registry, PERSONA_ID, "<esc>");

        // Then the picker scope is popped.
        assert!(
            state.frontend.picker_kind().is_none(),
            "close outcome must pop the picker scope"
        );
    }

    #[rstest::rstest]
    #[test]
    fn picker_close_is_applied_before_destination_push() {
        // Given an open picker and a destination scope.
        let mut state = state_with_persona_picker();
        let destination = SliceScopeId::new("picker-test", "destination");

        // When folding an outcome that closes the picker and pushes the destination.
        let result = fold(
            &mut state,
            PickerOutcome::empty()
                .close()
                .with_scope_signal(ScopeSignal::Push(destination.clone())),
        );

        // Then the destination is the sole overlay above the base scope.
        assert_eq!(
            result.scope_signal, None,
            "the applied signal should be consumed"
        );
        assert!(state.frontend.picker_kind().is_none());
        assert!(matches!(
            state.frontend.scope(),
            FocusScope::Dynamic(scope) if scope == destination
        ));
    }
}
