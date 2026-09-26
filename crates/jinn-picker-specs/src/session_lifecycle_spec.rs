//! The session-lifecycle picker's spec — behavior authored once in the builder.
//!
//! Open loads the implicit "blank" lifecycle plus every configured lifecycle
//! from `jinn.toml`, flagging those whose setup command takes `$`-parameters.
//! Enter either starts the session immediately (no-args lifecycles) or seeds
//! the lifecycle argument cell and returns a close-plus-push transition.

use jinn_picker::ActionCtx;
use jinn_picker::PickerId;
use jinn_picker::PickerOutcome;
use jinn_picker::PickerSpec;

use jinn_domain::common::app_state::AppState;
use jinn_domain::feat::ui::picker_states::PickerExt;
use jinn_preferences_config::ConfigLayer;
use jinn_preferences_config::schemas::LifecycleCommand;
use jinn_preferences_config::schemas::SessionLifecycle;
use jinn_session_lifecycle_msg::picker_entry::{SessionLifecycleEntry, lifecycle_row};
use jinn_session_lifecycle_msg::{ArgInputState, CommandTemplate};
use jinn_slices::ScopeSignal;

/// Builds the session-lifecycle picker's spec.
#[must_use]
pub fn session_lifecycle_spec() -> PickerSpec<SessionLifecycleEntry> {
    PickerSpec::new(PickerId::new(jinn_picker::SESSION_LIFECYCLE_ID))
        .title(" Session Lifecycle ")
        .row(lifecycle_row)
        .search(lifecycle_search_text)
        .on_open(open_lifecycle)
        .on_confirm(confirm_lifecycle)
}

/// The domain state behind an [`ActionCtx`]. The kernel's host lens always
/// lends `AppState`; this downcast is the spec's single sanctioned escape.
#[expect(
    clippy::expect_used,
    reason = "domain host lends AppState; a wrong downcast is a wiring bug"
)]
fn state_of<'a>(ctx: &'a mut ActionCtx<'_>) -> &'a mut AppState {
    ctx.state_any()
        .downcast_mut::<AppState>()
        .expect("domain host lends AppState")
}

/// The search text for one lifecycle row: name plus description. An absent
/// description contributes nothing but the separator space.
fn lifecycle_search_text(entry: &SessionLifecycleEntry) -> String {
    match &entry.description {
        Some(desc) => format!("{} {desc}", entry.name),
        None => entry.name.clone(),
    }
}

// ── Rendering ────────────────────────────────────────────────────────────

// ── Lifecycle ────────────────────────────────────────────────────────────

/// Opening the lifecycle picker: fresh filter + selection, then load the
/// implicit blank lifecycle plus every configured one. Opening never touches
/// the filesystem and emits no messages.
fn open_lifecycle(ctx: &mut ActionCtx<'_>) -> PickerOutcome {
    let config = ctx.config().clone();
    let state = state_of(ctx);
    state.frontend.session_lifecycle_picker_mut().reset();
    load_lifecycle_entries(state, &config);
    PickerOutcome::empty()
}

/// Enter on the lifecycle picker: start the session, or seed and open the
/// dynamic argument popup when the lifecycle's setup command needs parameters.
fn confirm_lifecycle(ctx: &mut ActionCtx<'_>) -> PickerOutcome {
    let Some(selected) = state_of(ctx)
        .frontend
        .session_lifecycle_picker()
        .selected_item()
        .map(|item| {
            let entry = item.entry();
            (entry.name.clone(), entry.has_args)
        })
    else {
        return PickerOutcome::empty();
    };
    let (lifecycle_name, has_args) = selected;
    let config = ctx.config().clone();
    let lifecycles = config.get_list::<SessionLifecycle>().unwrap_or_default();
    let state = state_of(ctx);

    if has_args {
        let Some(template) = lifecycles
            .iter()
            .find(|lifecycle| lifecycle.name == lifecycle_name)
            .and_then(|lifecycle| lifecycle.setup.as_ref())
            .and_then(|command| match command {
                LifecycleCommand::Shell(shell) => Some(CommandTemplate::parse(shell)),
                LifecycleCommand::Builtin(_) => None,
            })
        else {
            return PickerOutcome::empty();
        };
        let Some(cell) = state.frontend.slices().and_then(|slices| {
            slices.reader::<ArgInputState>(&jinn_session_lifecycle_msg::arg_input_slot())
        }) else {
            return PickerOutcome::empty();
        };
        let popup = ArgInputState::new(lifecycle_name, template);
        cell.update(|state| *state = popup);
        return PickerOutcome::empty()
            .close()
            .with_scope_signal(ScopeSignal::Push(
                jinn_session_lifecycle_msg::arg_input_scope(),
            ));
    }

    // No args - proceed directly. The setup function owns the scope
    // transition (clear overlays, push input), so this outcome carries no
    // close signal.
    let result = jinn_domain::feat::session_lifecycle::intent::handle_session_lifecycle_setup(
        state,
        &lifecycle_name,
        &[],
        None,
        &config,
    );
    PickerOutcome::from_route_result(result)
}

/// Loads lifecycle entries into the picker: the implicit blank lifecycle
/// first, then every configured lifecycle with its `has_args` flag detected
/// from the setup command's template parameters.
fn load_lifecycle_entries(state: &mut AppState, config: &ConfigLayer) {
    let mut entries = Vec::new();

    let theme = state.frontend.theme.clone();
    let lifecycles = config.get_list::<SessionLifecycle>().unwrap_or_default();

    // Always include the implicit blank lifecycle.
    entries.push(SessionLifecycleEntry {
        name: "blank".to_owned(),
        description: Some("New empty session".to_owned()),
        has_args: false,
        theme: theme.clone(),
    });

    // Add lifecycles from the `[[session_lifecycle.script]]` section.
    for lifecycle in &lifecycles {
        let has_args = lifecycle
            .setup
            .as_ref()
            .and_then(|cmd| match cmd {
                LifecycleCommand::Shell(s) => Some(s.as_str()),
                LifecycleCommand::Builtin(_) => None,
            })
            .is_some_and(|cmd| CommandTemplate::parse(cmd).has_params());
        entries.push(SessionLifecycleEntry {
            name: lifecycle.name.clone(),
            description: lifecycle.description.clone(),
            has_args,
            theme: theme.clone(),
        });
    }

    let wrapped = {
        let registry = crate::build_picker_registry();
        registry
            .make_items(jinn_picker::SESSION_LIFECYCLE_ID, entries)
            .unwrap_or_default()
    };
    state
        .frontend
        .session_lifecycle_picker_mut()
        .set_items(wrapped);
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test module, panics are acceptable"
    )]
    use super::*;
    use jinn_domain::PickerKind;
    use jinn_domain::common::app_state::AppState;
    use jinn_domain::feat::picker::host_impl::AppStatePickerHost;
    use jinn_picker::SESSION_LIFECYCLE_ID;
    use jinn_session_lifecycle_msg::arg_input_slot;
    use jinn_slices::FocusScope;

    /// A configuration layer whose `[[session_lifecycle.script]]` section
    /// declares the given lifecycles (name, description, setup-with-args).
    fn config_with_lifecycles(lifecycles: &[(&str, Option<&str>, Option<&str>)]) -> ConfigLayer {
        use std::fmt::Write as _;
        let mut document = String::new();
        for (name, description, setup) in lifecycles {
            let (name, description, setup) = (*name, *description, *setup);
            writeln!(document, "[[session_lifecycle.script]]\nname = \"{name}\"").expect("w");
            if let Some(description) = description {
                writeln!(document, "description = \"{description}\"").expect("w");
            }
            if let Some(setup) = setup {
                writeln!(document, "setup_command = \"{setup}\"").expect("w");
            }
        }
        jinn_config::testutil::config_layer(&document)
    }

    /// State with an active origin session and the lifecycle argument slot
    /// registered, over a document declaring `lifecycles` — the list the
    /// picker reads lives in config, not in the state snapshot.
    fn state_with_lifecycles(
        lifecycles: &[(&str, Option<&str>, Option<&str>)],
    ) -> (AppState, ConfigLayer) {
        let state = AppState::default_with_scope_focus();
        let slices = jinn_slices::Slices::new();
        slices
            .register(arg_input_slot(), ArgInputState::empty())
            .expect("fresh state registry has the lifecycle argument slot free");
        state.frontend.attach_slices(slices);
        (state, config_with_lifecycles(lifecycles))
    }

    /// Opens the picker through the real open path (scope push + spec open
    /// hook), mirroring what the intent handler does.
    fn open(state: &mut AppState, config: &ConfigLayer) {
        let registry = crate::build_picker_registry();
        jinn_domain::feat::picker::intent::handle_open_picker(
            state,
            PickerKind::SessionLifecycle,
            &registry,
            config,
        );
    }

    /// Runs a spec hook against `state` with a fresh dispatch context.
    fn run(
        state: &mut AppState,
        config: &ConfigLayer,
        f: impl FnOnce(&mut ActionCtx<'_>) -> PickerOutcome,
    ) -> PickerOutcome {
        let mut host = AppStatePickerHost::new(state, config);
        let mut ctx = ActionCtx::new(PickerId::new(SESSION_LIFECYCLE_ID), &mut host);
        f(&mut ctx)
    }

    // ── Open ─────────────────────────────────────────────────────────

    #[rstest::rstest]
    #[test]
    fn open_lists_blank_and_configured_lifecycles_with_args_flags() {
        // Given state with a parameterless lifecycle and a `$1` lifecycle.
        let (mut state, config) = state_with_lifecycles(&[
            ("plain", Some("No parameters"), None),
            ("templated", None, Some("cd /a/$1")),
        ]);

        // When opening the picker.
        open(&mut state, &config);

        // Then blank comes first, followed by the configured lifecycles.
        let items = state.frontend.session_lifecycle_picker().items();
        let names: Vec<&str> = items.iter().map(|i| i.entry().name.as_str()).collect();
        assert_eq!(names, vec!["blank", "plain", "templated"]);
        // And only the `$1` lifecycle is flagged has_args (blank never is).
        assert!(!items[0].entry().has_args);
        assert!(!items[1].entry().has_args);
        assert!(items[2].entry().has_args);
    }

    // ── Confirm, no args ─────────────────────────────────────────────

    #[rstest::rstest]
    #[test]
    fn confirm_on_empty_picker_is_a_no_op() {
        // Given an open picker whose entries were never populated (empty
        // registry test seam: the scope is active but storage is empty).
        let (mut state, config) = state_with_lifecycles(&[]);

        // When running the confirm hook.
        let outcome = run(&mut state, &config, confirm_lifecycle);

        // Then nothing happened: no session was created and no messages.
        assert_eq!(
            state.session.session_count(),
            1,
            "only the default origin exists"
        );
        assert!(outcome.messages.is_empty());
        assert!(!outcome.close);
    }

    #[rstest::rstest]
    #[test]
    fn confirm_without_args_starts_the_session() {
        // Given an open picker with the blank lifecycle selected.
        let (mut state, config) = state_with_lifecycles(&[]);
        open(&mut state, &config);

        // When confirming the selection.
        let outcome = run(&mut state, &config, confirm_lifecycle);

        // Then a second session was created and is active.
        assert_eq!(
            state.session.session_count(),
            2,
            "a new session was created"
        );
        // And the lifecycle name was stamped on the new session (the
        // "blank" pseudo-entry is stamped verbatim).
        assert_eq!(state.active_session().lifecycle_name(), Some("blank"));
        // And the setup messages were emitted for the new session.
        assert!(
            outcome
                .message_names
                .iter()
                .any(|n| n.ends_with("SessionCreated")),
            "SessionCreated must be emitted: {:?}",
            outcome.message_names
        );
        // And the picker closed by clearing overlays (input scope active).
        assert_eq!(
            state.frontend.scope(),
            FocusScope::Input,
            "setup transitions to the input scope"
        );
    }

    // PINNED: the spec's confirm hook passes the shared empty layer to the
    // session-lifecycle setup rather than its own, so the selected
    // lifecycle's setup command never resolves.
    #[rstest::rstest]
    #[test]
    fn confirm_scripted_lifecycle_without_params_stamps_and_runs_setup() {
        // Given an open picker with a no-args scripted lifecycle selected.
        let (mut state, config) =
            state_with_lifecycles(&[("research", Some("Research setup"), Some("echo ready"))]);
        open(&mut state, &config);
        state.frontend.session_lifecycle_picker_mut().move_down(1);

        // When confirming.
        let outcome = run(&mut state, &config, confirm_lifecycle);

        // Then the new session carries the lifecycle name.
        assert_eq!(state.active_session().lifecycle_name(), Some("research"),);
        // And the setup run message was emitted.
        assert!(
            outcome
                .message_names
                .iter()
                .any(|n| n.ends_with("RunSessionSetup")),
            "RunSessionSetup must be emitted: {:?}",
            outcome.message_names
        );
    }

    // ── Confirm, with args ───────────────────────────────────────────

    #[rstest::rstest]
    #[test]
    fn confirm_with_args_seeds_popup_and_signals_destination() {
        // Given an open picker with a `$1` lifecycle selected.
        let (mut state, config) = state_with_lifecycles(&[
            ("project-a", None, Some("cd /a/$1")),
            ("project-b", None, Some("cd /b/$1")),
        ]);
        open(&mut state, &config);
        // blank -> project-a -> project-b (move_down is single-step).
        state.frontend.session_lifecycle_picker_mut().move_down(1);
        state.frontend.session_lifecycle_picker_mut().move_down(1);

        // When confirming.
        let outcome = run(&mut state, &config, confirm_lifecycle);

        // Then the registered cell holds the selected lifecycle and parsed template.
        let cell = state
            .frontend
            .slices()
            .and_then(|slices| slices.reader::<ArgInputState>(&arg_input_slot()))
            .expect("test state registered the lifecycle argument cell");
        assert_eq!(cell.read().lifecycle_name, "project-b");
        assert!(cell.read().template.display().contains("/b/"));
        assert!(cell.read().text.input.is_empty());
        // And the outcome requests close-then-push without mutating scope directly.
        assert!(outcome.close);
        assert_eq!(
            outcome.scope_signal,
            Some(ScopeSignal::Push(
                jinn_session_lifecycle_msg::arg_input_scope()
            ))
        );
        assert_eq!(
            state.frontend.scope(),
            FocusScope::Picker {
                kind: PickerKind::SessionLifecycle
            }
        );
        assert!(outcome.messages.is_empty());
    }
}
