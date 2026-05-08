//! Session picker handler — processes `SessionNew` and `SessionLoadCompleted` commands.
//!
//! `SessionNew` creates a fresh session and closes the picker.
//! `SessionLoadCompleted` restores a session from persisted data.

use nullslop_component_core::{HandlerContext, define_handler};
use nullslop_protocol as npr;
use nullslop_protocol::{
    CommandAction, PickerKind, SessionId, SessionLoadCompleted, SessionNew,
};
use nullslop_protocol::context::{RestoreStrategyState, SwitchPromptStrategy};
use nullslop_protocol::system::SetMode;
use nullslop_session::{BLOB_STRATEGY_STATE, BLOB_WORKFLOW_STATE};
use nullslop_services::Services;
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

        ctx.state
            .sessions
            .insert(cmd.session_id.clone(), session);
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

    #[test]
    fn session_new_creates_fresh_session_and_closes_picker() {
        // Given a bus with SessionPickerHandler and session picker active.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            active_picker_kind: Some(PickerKind::Session),
            mode: Mode::Picker,
            ..crate::AppState::default()
        };
        let old_session = state.active_session.clone();

        // When processing SessionNew.
        bus.submit_command(nullslop_protocol::Command::SessionNew);
        bus.process_commands(&mut state, &services);

        // Then a new session is created and set as active.
        assert_ne!(state.active_session, old_session);
        // And mode is back to Normal (via SetMode).
        assert_eq!(state.mode, Mode::Normal);
    }

    #[test]
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

    #[test]
    fn session_load_completed_restores_session() {
        // Given a bus with SessionPickerHandler and session_loading = true.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            session_loading: true,
            ..crate::AppState::default()
        };

        let session_id = SessionId::new();
        let cmd = SessionLoadCompleted {
            session_id: session_id.clone(),
            title: "Test Session".to_owned(),
            history: vec![ChatEntry::user("hello"), ChatEntry::assistant("world")],
            active_strategy: PromptStrategyId::passthrough(),
            blobs: HashMap::new(),
        };

        // When processing SessionLoadCompleted.
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted {
            payload: cmd,
        });
        bus.process_commands(&mut state, &services);

        // Then session_loading is cleared.
        assert!(!state.session_loading);
        // And the active session is the loaded one.
        assert_eq!(state.active_session, session_id);
        // And history is restored.
        assert_eq!(state.active_session().history().len(), 2);
    }

    #[test]
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
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted {
            payload: cmd,
        });
        bus.process_commands(&mut state, &services);

        // Then session_loading is false.
        assert!(!state.session_loading);
    }

    #[test]
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
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted {
            payload: cmd,
        });
        bus.process_commands(&mut state, &services);

        // Then a SwitchPromptStrategy command is emitted.
        let commands = bus.drain_processed_commands();
        let switch_cmd = commands.iter().find_map(|c| match &c.command {
            nullslop_protocol::Command::SwitchPromptStrategy { payload } => Some(payload.clone()),
            _ => None,
        });
        assert!(switch_cmd.is_some(), "expected SwitchPromptStrategy command");
        let switch_cmd = switch_cmd.expect("should have SwitchPromptStrategy");
        assert_eq!(switch_cmd.session_id, session_id);
        assert_eq!(switch_cmd.strategy_id, PromptStrategyId::sliding_window());
    }

    #[test]
    fn session_load_completed_emits_restore_strategy_state_when_blob_present() {
        // Given a bus with SessionPickerHandler registered.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = crate::AppState {
            session_loading: true,
            ..crate::AppState::default()
        };

        let session_id = SessionId::new();
        let blob = serde_json::json!({"compaction_count": 5});
        let mut blobs = HashMap::new();
        blobs.insert("strategy_state".to_owned(), blob.clone());

        let cmd = SessionLoadCompleted {
            session_id: session_id.clone(),
            title: String::new(),
            history: vec![],
            active_strategy: PromptStrategyId::compaction(),
            blobs,
        };

        // When processing SessionLoadCompleted with a strategy blob.
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted {
            payload: cmd,
        });
        bus.process_commands(&mut state, &services);

        // Then a RestoreStrategyState command is emitted.
        let commands = bus.drain_processed_commands();
        let restore_cmd = commands.iter().find_map(|c| match &c.command {
            nullslop_protocol::Command::RestoreStrategyState { payload } => Some(payload.clone()),
            _ => None,
        });
        assert!(restore_cmd.is_some(), "expected RestoreStrategyState command");
        let restore_cmd = restore_cmd.expect("should have RestoreStrategyState");
        assert_eq!(restore_cmd.session_id, session_id);
        assert_eq!(restore_cmd.strategy_id, PromptStrategyId::compaction());
        assert_eq!(restore_cmd.blob, blob);
    }

    #[test]
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
        bus.submit_command(nullslop_protocol::Command::SessionLoadCompleted {
            payload: cmd,
        });
        bus.process_commands(&mut state, &services);

        // Then no RestoreStrategyState command is emitted.
        let commands = bus.drain_processed_commands();
        let has_restore = commands.iter().any(|c| {
            matches!(&c.command, nullslop_protocol::Command::RestoreStrategyState { .. })
        });
        assert!(!has_restore, "should not emit RestoreStrategyState without blob");
    }
}
