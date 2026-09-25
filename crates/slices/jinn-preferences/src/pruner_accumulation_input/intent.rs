//! Route actions and input hook for the pruner accumulation threshold popup.

use jinn_preferences_config::protocol::command::{PreferenceUpdate, UpdatePreferences};
use jinn_slices::LineInput;
use jinn_slices::RouteResult;
use jinn_slices::SliceScopeId;
use jinn_slices::SlotKey;
use jinn_slices::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteId, RouteOutcome, RouteRow,
    ScopeSignal,
};

use super::state::PrunerAccumulationInputState;

/// The popup's dynamic input-capturing scope.
#[must_use]
pub fn pruner_accumulation_scope() -> SliceScopeId {
    SliceScopeId::new("preferences", "pruner_accumulation_input")
}

/// The slot containing the popup's editable threshold state.
#[must_use]
pub fn pruner_accumulation_slot() -> SlotKey {
    SlotKey::builtin("preferences", "pruner_accumulation_input")
}

type PrunerCell = TypedCell<PrunerAccumulationInputState>;

fn action<F>(cell: &PrunerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &PrunerCell) -> RouteResult + Send + Sync + 'static,
{
    let cell = cell.clone();
    ActionFn::new(move |mut ctx| f(&mut ctx, &cell))
}

fn popup_row(
    action_name: &'static str,
    key: &'static str,
    display: &'static str,
    run: ActionFn,
) -> RouteRow {
    RouteRow {
        route_id: RouteId::new(action_name),
        scope: pruner_accumulation_scope(),
        key,
        category: "preferences",
        site: BindSite::OwnScope,
        feature: "preferences",
        outcome: RouteOutcome::Action {
            action: action_name,
            display,
            run,
        },
    }
}

/// Attaches the normal-mode opener and the popup's confirm/leave/Ctrl-C rows.
pub fn attach_pruner_accumulation_rows(routes: &jinn_slices::KeyRoutes, cell: &PrunerCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("preferences:open-pruner-accumulation"),
        scope: pruner_accumulation_scope(),
        key: "gcp",
        category: "preferences",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "preferences",
        outcome: RouteOutcome::Action {
            action: "open-pruner-accumulation",
            display: "set pruner accumulation threshold",
            run: action(cell, open_pruner_accumulation),
        },
    });
    routes.attach(popup_row(
        "confirm-pruner-accumulation",
        "<enter>",
        "save the pruner accumulation threshold",
        action(cell, confirm_pruner_accumulation),
    ));
    routes.attach(popup_row(
        "leave-pruner-accumulation",
        "<esc>",
        "cancel threshold editing",
        action(cell, |_ctx, cell| {
            leave_pruner_accumulation(cell);
            RouteResult::empty().with_scope_signal(ScopeSignal::PopIf(pruner_accumulation_scope()))
        }),
    ));
    routes.attach(popup_row(
        "clear-or-leave-pruner-accumulation",
        "<c-c>",
        "clear the threshold, or leave when already empty",
        action(cell, |_ctx, cell| clear_or_leave_pruner_accumulation(cell)),
    ));
}

/// Registers the editing hook for the popup's dynamic scope.
pub fn register_pruner_accumulation_input_hook(routes: &jinn_slices::KeyRoutes, cell: &PrunerCell) {
    let cell = cell.clone();
    let hook: InputHook = std::sync::Arc::new(move |intent: &EditIntent| {
        let cell = cell.clone();
        match intent {
            EditIntent::InsertChar(ch) if ch.is_ascii_digit() => {
                cell.update(|state| state.text.insert_char(*ch));
            }
            EditIntent::InsertChar(_) => {}
            EditIntent::DeleteBackward => {
                cell.update(|state| state.text.delete());
            }
            EditIntent::DeleteForward => {
                cell.update(|state| state.text.delete_forward());
            }
            EditIntent::CursorLeft => {
                cell.update(|state| state.text.cursor_left());
            }
            EditIntent::CursorRight => {
                cell.update(|state| state.text.cursor_right());
            }
            EditIntent::CursorHome => {
                cell.update(|state| state.text.cursor_home());
            }
            EditIntent::CursorEnd => {
                cell.update(|state| state.text.cursor_end());
            }
            EditIntent::Paste(text) => {
                let digits: String = text.chars().filter(char::is_ascii_digit).collect();
                cell.update(|state| state.text.paste(&digits));
            }
        }
        Some(RouteResult::empty())
    });
    routes.register_input_hook(&pruner_accumulation_scope(), hook);
}

/// Seeds the popup from the current threshold and requests its dynamic scope.
fn open_pruner_accumulation(ctx: &mut ActionCtx<'_>, cell: &PrunerCell) -> RouteResult {
    let Some(threshold) = ctx.state.as_any_mut().and_then(|state| {
        state.downcast_mut::<jinn_domain::AppState>().map(|state| {
            state
                .frontend
                .preferences
                .auto_prune
                .accumulation_threshold_tokens
        })
    }) else {
        return RouteResult::empty();
    };
    let mut text = LineInput::new();
    text.set(threshold.to_string());
    cell.update(|state| state.text = text);
    RouteResult::empty().with_scope_signal(ScopeSignal::Push(pruner_accumulation_scope()))
}

/// Confirms a valid decimal threshold and emits the authoritative preference update.
fn confirm_pruner_accumulation(_ctx: &mut ActionCtx<'_>, cell: &PrunerCell) -> RouteResult {
    let raw = cell.read().text.input.trim().to_owned();
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return RouteResult::empty();
    }
    let Ok(threshold) = raw.parse::<u32>() else {
        return RouteResult::empty();
    };
    leave_pruner_accumulation(cell);
    RouteResult::new_message(UpdatePreferences {
        updates: vec![PreferenceUpdate::SetAccumulationThreshold(threshold)],
    })
    .with_scope_signal(ScopeSignal::PopIf(pruner_accumulation_scope()))
}

/// Clears the popup state before any close transition.
fn leave_pruner_accumulation(cell: &PrunerCell) {
    cell.update(|state| *state = PrunerAccumulationInputState::default());
}

/// Clears nonempty input, or leaves an already empty popup.
fn clear_or_leave_pruner_accumulation(cell: &PrunerCell) -> RouteResult {
    let had_text = !cell.read().text.input.is_empty();
    leave_pruner_accumulation(cell);
    if had_text {
        RouteResult::empty()
    } else {
        RouteResult::empty().with_scope_signal(ScopeSignal::PopIf(pruner_accumulation_scope()))
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        reason = "test code, fresh-registry setup is infallible"
    )]
    use super::*;
    use jinn_slices::route::EditIntent;

    fn state() -> jinn_domain::AppState {
        jinn_domain::AppState::default_with_scope_focus()
    }

    fn cell() -> (jinn_slices::Slices, PrunerCell) {
        let slices = jinn_slices::Slices::new();
        let cell = slices
            .register(
                pruner_accumulation_slot(),
                PrunerAccumulationInputState::default(),
            )
            .expect("fresh registry has the pruner slot free");
        (slices, cell)
    }

    #[rstest::rstest]
    fn opener_seeds_current_threshold_at_cursor_end() {
        // Given a popup cell and state carrying a threshold.
        let (slices, cell) = cell();
        let mut state = state();
        state
            .frontend
            .preferences
            .auto_prune
            .accumulation_threshold_tokens = 7_500;
        let mut ctx = ActionCtx {
            state: &mut state,
            slices: &slices,
            key_bytes: Vec::new(),
        };

        // When the opener runs.
        let result = open_pruner_accumulation(&mut ctx, &cell);

        // Then it seeds the editable value and requests the dynamic scope.
        assert_eq!(cell.read().text.input, "7500");
        assert_eq!(cell.read().text.cursor_pos, 4);
        assert_eq!(
            result.scope_signal,
            Some(ScopeSignal::Push(pruner_accumulation_scope()))
        );
    }

    #[rstest::rstest]
    fn input_hook_rejects_non_digits_and_filters_paste() {
        // Given a route table with the popup hook.
        let (_slices, cell) = cell();
        let routes = jinn_slices::KeyRoutes::new();
        register_pruner_accumulation_input_hook(&routes, &cell);
        let hook = routes
            .input_hook(&pruner_accumulation_scope())
            .expect("hook registered");

        // When text and mixed pasted content arrive.
        hook(&EditIntent::InsertChar('a'));
        hook(&EditIntent::InsertChar('2'));
        hook(&EditIntent::Paste("3x4".to_owned()));

        // Then only ASCII digits are inserted.
        assert_eq!(cell.read().text.input, "234");
    }

    #[rstest::rstest]
    fn valid_confirm_publishes_update_and_pops() {
        // Given a valid threshold in the popup cell.
        let (slices, cell) = cell();
        cell.update(|state| state.text.set("25000".to_owned()));
        let mut state = state();
        let mut ctx = ActionCtx {
            state: &mut state,
            slices: &slices,
            key_bytes: Vec::new(),
        };

        // When confirmation runs.
        let result = confirm_pruner_accumulation(&mut ctx, &cell);

        // Then it publishes one update and requests the popup to close.
        assert_eq!(result.message_names, ["UpdatePreferences"]);
        assert_eq!(
            result.scope_signal,
            Some(ScopeSignal::PopIf(pruner_accumulation_scope()))
        );
        assert!(cell.read().text.input.is_empty());
    }

    #[rstest::rstest]
    fn overflow_confirm_keeps_popup_open() {
        // Given a digit-only value outside the u32 range.
        let (slices, cell) = cell();
        cell.update(|state| state.text.set("4294967296".to_owned()));
        let mut state = state();
        let mut ctx = ActionCtx {
            state: &mut state,
            slices: &slices,
            key_bytes: Vec::new(),
        };

        // When confirmation runs.
        let result = confirm_pruner_accumulation(&mut ctx, &cell);

        // Then it consumes Enter without publishing or closing.
        assert!(result.message_names.is_empty());
        assert!(result.scope_signal.is_none());
        assert_eq!(cell.read().text.input, "4294967296");
    }
}
