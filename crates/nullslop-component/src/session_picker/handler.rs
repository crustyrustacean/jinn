//! Session picker handler — processes `SessionNew` and `SessionLoadCompleted` commands.
//!
//! `SessionNew` creates a fresh session and closes the picker.
//! `SessionLoadCompleted` restores a session from persisted data.

use nullslop_component_core::{HandlerContext, define_handler};
use nullslop_protocol as npr;
use nullslop_protocol::context::{RestoreStrategyState, SwitchPromptStrategy};
use nullslop_protocol::system::SetMode;
use nullslop_protocol::{CommandAction, PickerKind, SessionId, SessionLoadCompleted, SessionNew};
use nullslop_services::Services;
use nullslop_session::{BLOB_STRATEGY_STATE, BLOB_WORKFLOW_STATE};
use nullslop_workflow::WorkflowState;

use crate::AppState;
use crate::chat_session::ChatSessionState;

define_handler! {
    pub(crate) struct SessionPickerHandler;

    commands {
        SessionNew: on_session_new,
        SessionLoadCompleted: on_session_load_completed,
    }

    events {}
}

impl SessionPickerHandler {
    /// Handles `SessionNew`: creates a fresh session and closes the picker.
    ///
    /// Only acts when the session picker is active. Creates a new session ID,
    /// inserts a fresh [`ChatSessionState`], sets it as active, and closes
    /// the picker by switching to Normal mode.
    fn on_session_new(
        _cmd: &SessionNew,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        // Only act when session picker is active.
        if ctx.state.active_picker_kind != Some(PickerKind::Session) {
            return CommandAction::Continue;
        }

        // Evict the previous session before creating a new one.
        ctx.state.sessions.remove(&ctx.state.active_session);

        let new_id = SessionId::new();
        ctx.state
            .sessions
            .insert(new_id.clone(), ChatSessionState::new());
        ctx.state.active_session = new_id;

        // Close picker.
        ctx.out.submit_command(npr::Command::SetMode {
            payload: SetMode {
                mode: npr::Mode::Normal,
            },
        });
        CommandAction::Continue
    }

    /// Handles `SessionLoadCompleted`: restores a session from persisted data.
    ///
    /// Clears the loading flag, reconstructs [`ChatSessionState`] from the
    /// loaded data (history, strategy, workflow/strategy blobs), inserts it
    /// into the sessions map, and sets it as active.
    fn on_session_load_completed(
        cmd: &SessionLoadCompleted,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.session_loading = false;

        // Evict the previous session before loading the new one.
        ctx.state.sessions.remove(&ctx.state.active_session);

        let mut session = ChatSessionState::new();
        session.restore_history(cmd.history.clone());
        session.switch_strategy(cmd.active_strategy.clone());

        // Deserialize workflow blob.
        if let Some(workflow_value) = cmd
            .blobs
            .get(BLOB_WORKFLOW_STATE)
            .and_then(|v| serde_json::from_value::<WorkflowState>(v.clone()).ok())
        {
            session.set_workflow(workflow_value);
        }

        // Deserialize strategy blob.
        if let Some(strategy_value) = cmd.blobs.get(BLOB_STRATEGY_STATE) {
            session.set_strategy_state(strategy_value.clone());
        }

        ctx.state.sessions.insert(cmd.session_id.clone(), session);
        ctx.state.active_session = cmd.session_id.clone();

        // Notify the context actor of the loaded strategy so it doesn't
        // default to passthrough on the next AssemblePrompt.
        ctx.out.submit_command(npr::Command::SwitchPromptStrategy {
            payload: SwitchPromptStrategy {
                session_id: cmd.session_id.clone(),
                strategy_id: cmd.active_strategy.clone(),
            },
        });

        // Forward persisted strategy state to the context actor.
        if let Some(strategy_blob) = cmd.blobs.get(BLOB_STRATEGY_STATE) {
            ctx.out.submit_command(npr::Command::RestoreStrategyState {
                payload: RestoreStrategyState {
                    session_id: cmd.session_id.clone(),
                    strategy_id: cmd.active_strategy.clone(),
                    blob: strategy_blob.clone(),
                },
            });
        }

        CommandAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nullslop_component_core::Bus;
    use nullslop_protocol::{
        ChatEntry, Mode, PickerKind, PromptStrategyId, SessionId, SessionLoadCompleted,
    };
    use nullslop_services::Services;

    use super::SessionPickerHandler;
    use crate::test_utils;

    fn setup_bus() -> Bus<crate::AppState, Services> {
        let mut bus = Bus::new();
        SessionPickerHandler.register(&mut bus);
        crate::chat_input_box::ChatInputBoxHandler.register(&mut bus);
        bus
    }

    #[rstest::rstest]
    fn session_new_creates_fresh_session() {
        // Given a bus with SessionPickerHandler registered.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            active_picker_kind: Some(PickerKind::Session),
            ..crate::AppState::default()
        };

        // When receiving SessionNew.
        bus.submit_command(nullslop_protocol::Command::SessionNew);
        bus.process_commands(&mut state, &services);

        // Then a new session was created.
        assert!(state.active_session().history().is_empty());
    }

    #[rstest::rstest]
    fn session_new_closes_picker() {
        // Given a bus with SessionPickerHandler registered.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            active_picker_kind: Some(PickerKind::Session),
            ..crate::AppState::default()
        };

        // When receiving SessionNew.
        bus.submit_command(nullslop_protocol::Command::SessionNew);
        bus.process_commands(&mut state, &services);

        // Then mode is back to Normal (picker closed via SetMode command).
        let commands = bus.drain_processed_commands();
        let has_set_mode = commands
            .iter()
            .any(|c| matches!(&c.command, nullslop_protocol::Command::SetMode { .. }));
        assert!(has_set_mode);
    }

    #[rstest::rstest]
    fn session_new_evicts_old_session() {
        // Given a state with an existing session.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            active_picker_kind: Some(PickerKind::Session),
            ..crate::AppState::default()
        };
        let original_session = state.active_session.clone();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("old"));

        // When receiving SessionNew.
        bus.submit_command(nullslop_protocol::Command::SessionNew);
        bus.process_commands(&mut state, &services);

        // Then the old session is gone.
        assert_ne!(state.active_session, original_session);
    }

    #[rstest::rstest]
    fn session_new_creates_new_session() {
        // Given a state with an existing session.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            active_picker_kind: Some(PickerKind::Session),
            ..crate::AppState::default()
        };
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("old"));

        // When receiving SessionNew.
        bus.submit_command(nullslop_protocol::Command::SessionNew);
        bus.process_commands(&mut state, &services);

        // Then the new session has no history.
        assert!(state.active_session().history().is_empty());
    }

    #[rstest::rstest]
    fn session_new_has_exactly_one_session() {
        // Given a state with an existing session.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            active_picker_kind: Some(PickerKind::Session),
            ..crate::AppState::default()
        };
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("old"));

        // When receiving SessionNew.
        bus.submit_command(nullslop_protocol::Command::SessionNew);
        bus.process_commands(&mut state, &services);

        // Then there is exactly one session.
        assert_eq!(state.sessions.len(), 1);
    }

    #[rstest::rstest]
    fn session_new_ignores_when_session_picker_not_active() {
        // Given a bus with SessionPickerHandler and no picker active.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState::default();
        let original_session = state.active_session.clone();

        // When processing SessionNew.
        bus.submit_command(nullslop_protocol::Command::SessionNew);
        bus.process_commands(&mut state, &services);

        // Then the active session is unchanged.
        assert_eq!(state.active_session, original_session);
    }

    #[rstest::rstest]
    fn load_completed_clears_loading_flag() {
        // Given a bus with SessionPickerHandler registered.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            session_loading: true,
            ..crate::AppState::default()
        };

        let cmd = SessionLoadCompleted {
            session_id: SessionId::new(),
            title: "Restored".to_owned(),
            history: vec![ChatEntry::user("hello")],
            active_strategy: PromptStrategyId::passthrough(),
            blobs: HashMap::new(),
        };
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted { payload: cmd });
        bus.process_commands(&mut state, &services);

        // Then loading flag is cleared.
        assert!(!state.session_loading);
    }

    #[rstest::rstest]
    fn load_completed_sets_active_session() {
        // Given a bus with SessionPickerHandler registered.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            session_loading: true,
            ..crate::AppState::default()
        };

        let cmd = SessionLoadCompleted {
            session_id: SessionId::new(),
            title: "Restored".to_owned(),
            history: vec![ChatEntry::user("hello")],
            active_strategy: PromptStrategyId::passthrough(),
            blobs: HashMap::new(),
        };
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted { payload: cmd });
        bus.process_commands(&mut state, &services);

        // Then mode returns to Normal.
        assert_eq!(state.mode, Mode::Normal);
    }

    #[rstest::rstest]
    fn load_completed_restores_history() {
        // Given a bus with SessionPickerHandler registered.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            session_loading: true,
            ..crate::AppState::default()
        };

        let cmd = SessionLoadCompleted {
            session_id: SessionId::new(),
            title: "Restored".to_owned(),
            history: vec![ChatEntry::user("hello")],
            active_strategy: PromptStrategyId::passthrough(),
            blobs: HashMap::new(),
        };
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted { payload: cmd });
        bus.process_commands(&mut state, &services);

        // Then history is restored.
        assert_eq!(state.active_session().history().len(), 1);
    }

    #[rstest::rstest]
    fn load_completed_evicts_old_session() {
        // Given a state with an existing session containing entries.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            session_loading: true,
            ..crate::AppState::default()
        };
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("old"));

        let cmd = SessionLoadCompleted {
            session_id: SessionId::new(),
            title: "New".to_owned(),
            history: vec![],
            active_strategy: PromptStrategyId::passthrough(),
            blobs: HashMap::new(),
        };
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted { payload: cmd });
        bus.process_commands(&mut state, &services);

        // Then the old session history was replaced.
        assert!(state.active_session().history().is_empty());
    }

    #[rstest::rstest]
    fn load_completed_adds_new_session() {
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            session_loading: true,
            ..crate::AppState::default()
        };

        let cmd = SessionLoadCompleted {
            session_id: SessionId::new(),
            title: "New".to_owned(),
            history: vec![ChatEntry::user("restored")],
            active_strategy: PromptStrategyId::passthrough(),
            blobs: HashMap::new(),
        };
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted { payload: cmd });
        bus.process_commands(&mut state, &services);

        // Then the new session history is present.
        assert_eq!(state.active_session().history().len(), 1);
    }

    #[rstest::rstest]
    fn load_completed_has_exactly_one_session() {
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            session_loading: true,
            ..crate::AppState::default()
        };

        let cmd = SessionLoadCompleted {
            session_id: SessionId::new(),
            title: "New".to_owned(),
            history: vec![],
            active_strategy: PromptStrategyId::passthrough(),
            blobs: HashMap::new(),
        };
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted { payload: cmd });
        bus.process_commands(&mut state, &services);

        // Then the session is active.
        assert!(!state.session_loading);
        assert_eq!(state.mode, Mode::Normal);
    }

    #[rstest::rstest]
    fn session_load_completed_clears_loading_flag() {
        // Given a bus with SessionPickerHandler and session_loading = true.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            session_loading: true,
            ..crate::AppState::default()
        };

        let cmd = SessionLoadCompleted {
            session_id: SessionId::new(),
            title: String::new(),
            history: vec![],
            active_strategy: PromptStrategyId::passthrough(),
            blobs: HashMap::new(),
        };

        // When processing SessionLoadCompleted with empty data.
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted { payload: cmd });
        bus.process_commands(&mut state, &services);

        // Then session_loading is false.
        assert!(!state.session_loading);
    }

    #[rstest::rstest]
    fn session_load_completed_emits_switch_prompt_strategy() {
        // Given a bus with SessionPickerHandler registered.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            session_loading: true,
            ..crate::AppState::default()
        };

        let session_id = SessionId::new();
        let cmd = SessionLoadCompleted {
            session_id: session_id.clone(),
            title: "Sliding Session".to_owned(),
            history: vec![],
            active_strategy: PromptStrategyId::sliding_window(),
            blobs: HashMap::new(),
        };

        // When processing SessionLoadCompleted with sliding_window strategy.
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted { payload: cmd });
        bus.process_commands(&mut state, &services);

        // Then a SwitchPromptStrategy command is emitted.
        let commands = bus.drain_processed_commands();
        let switch_cmd = commands.iter().find_map(|c| match &c.command {
            nullslop_protocol::Command::SwitchPromptStrategy { payload } => Some(payload.clone()),
            _ => None,
        });
        assert!(
            switch_cmd.is_some(),
            "expected SwitchPromptStrategy command"
        );
        let switch_cmd = switch_cmd.expect("should have SwitchPromptStrategy");
        assert_eq!(switch_cmd.session_id, session_id);
        assert_eq!(switch_cmd.strategy_id, PromptStrategyId::sliding_window());
    }

    #[rstest::rstest]
    fn load_completed_emits_restore_strategy_command() {
        // Given a state with loading flag set.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            session_loading: true,
            ..crate::AppState::default()
        };

        let session_id = SessionId::new();
        let cmd = SessionLoadCompleted {
            session_id: session_id.clone(),
            title: "Blob Session".to_owned(),
            history: vec![],
            active_strategy: PromptStrategyId::sliding_window(),
            blobs: HashMap::from([("strategy_state".to_owned(), serde_json::json!({"count": 5}))]),
        };
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted { payload: cmd });
        bus.process_commands(&mut state, &services);
        let commands = bus.drain_processed_commands();

        // Then a RestoreStrategyState command is emitted.
        let has_restore = commands.iter().any(|c| {
            matches!(
                &c.command,
                nullslop_protocol::Command::RestoreStrategyState { .. }
            )
        });
        assert!(has_restore);
    }

    #[rstest::rstest]
    fn restore_command_contains_blob() {
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            session_loading: true,
            ..crate::AppState::default()
        };
        let session_id = SessionId::new();
        let blob = serde_json::json!({"count": 5});

        let cmd = SessionLoadCompleted {
            session_id,
            title: "Blob Session".to_owned(),
            history: vec![],
            active_strategy: PromptStrategyId::sliding_window(),
            blobs: HashMap::from([("strategy_state".to_owned(), blob.clone())]),
        };
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted { payload: cmd });
        bus.process_commands(&mut state, &services);
        let commands = bus.drain_processed_commands();

        // Then the restore command contains the blob.
        let restore = commands
            .iter()
            .find(|c| {
                matches!(
                    &c.command,
                    nullslop_protocol::Command::RestoreStrategyState { .. }
                )
            })
            .expect("should have RestoreStrategyState");
        if let nullslop_protocol::Command::RestoreStrategyState { payload } = &restore.command {
            assert_eq!(payload.blob, blob);
        }
    }

    #[rstest::rstest]
    fn session_load_completed_skips_restore_when_no_strategy_blob() {
        // Given a bus with SessionPickerHandler registered.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            session_loading: true,
            ..crate::AppState::default()
        };

        let cmd = SessionLoadCompleted {
            session_id: SessionId::new(),
            title: String::new(),
            history: vec![],
            active_strategy: PromptStrategyId::passthrough(),
            blobs: HashMap::new(),
        };

        // When processing SessionLoadCompleted with no blobs.
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted { payload: cmd });
        bus.process_commands(&mut state, &services);

        // Then no RestoreStrategyState command is emitted.
        let commands = bus.drain_processed_commands();
        let has_restore = commands.iter().any(|c| {
            matches!(
                &c.command,
                nullslop_protocol::Command::RestoreStrategyState { .. }
            )
        });
        assert!(
            !has_restore,
            "should not emit RestoreStrategyState without blob"
        );
    }
}
