//! Host-side handler for Lua VM requests.
//!
//! [`LuaHostHandler`] receives [`HostRequest`] from the Lua VM channel
//! and processes each variant against the domain layer:
//!
//! - `Llm` → calls the LLM via [`DomainNodeContext`]
//! - `PushUser` → pushes a [`ChatEntry::user`] into session history
//! - `PushSystem` → pushes a [`ChatEntry::system`] into session history
//! - `TurnOff` → disables an attached workflow

use std::sync::Arc;

use jinn_lua_workflow::HostRequest;
use error_stack::{Report, ResultExt};
use wherror::Error;

use crate::common::state::State;
use crate::feat::session::chat_entry::ChatEntry;
use crate::feat::workflow::attached_workflow::AttachedWorkflowState;
use crate::feat::workflow::domain_node_context::DomainNodeContext;
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::protocol::Command;

/// Error type for Lua host handler operations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct LuaHostHandlerError;

/// Host-side handler for Lua VM requests.
///
/// Processes [`HostRequest`] variants against the domain layer.
/// Holds shared state for accessing sessions and workflow attachments,
/// and a [`DomainNodeContext`] for LLM calls.
#[derive(Clone)]
pub struct LuaHostHandler {
    /// Shared application state.
    state: State,
    /// Domain node context for LLM access.
    ctx: Arc<DomainNodeContext>,
}

impl LuaHostHandler {
    /// Creates a new host handler.
    pub fn new(state: State, ctx: Arc<DomainNodeContext>) -> Self {
        Self { state, ctx }
    }

    /// Handles a single host request asynchronously.
    ///
    /// Processes the request and sends the response through the
    /// oneshot channel included in the request variant.
    pub async fn handle_request(&self, request: HostRequest) {
        match request {
            HostRequest::Llm {
                session_id,
                prompt,
                system_prompt,
                respond_to,
            } => {
                let result = self
                    .handle_llm(&session_id, &prompt, system_prompt.as_deref())
                    .await;
                let _ = respond_to.send(result.map_err(|r| format!("{r:#}")));
            }
            HostRequest::PushUser {
                session_id,
                text,
                respond_to,
            } => {
                let result = self.handle_push_user(&session_id, &text);
                let _ = respond_to.send(result.map_err(|r| format!("{r:#}")));
            }
            HostRequest::PushSystem {
                session_id,
                text,
                respond_to,
            } => {
                let result = self.handle_push_system(&session_id, &text);
                let _ = respond_to.send(result.map_err(|r| format!("{r:#}")));
            }
            HostRequest::TurnOff {
                workflow_id,
                respond_to,
            } => {
                let result = self.handle_turn_off(&workflow_id);
                let _ = respond_to.send(result.map_err(|r| format!("{r:#}")));
            }
            HostRequest::Shutdown => {
                // Nothing to do — the VM task will exit.
            }
        }
    }

    /// Runs the host handler loop, processing requests until the channel closes.
    pub async fn run(self, receiver: kanal::Receiver<HostRequest>) {
        while let Ok(request) = receiver.recv() {
            if matches!(request, HostRequest::Shutdown) {
                break;
            }
            self.handle_request(request).await;
        }
        tracing::debug!("lua host handler shutting down");
    }

    /// Handles an LLM request.
    ///
    /// Calls [`DomainNodeContext::send_llm_request_cloned`] to invoke the LLM
    /// in a cloned session. The response is returned through the oneshot.
    ///
    /// # Errors
    ///
    /// Returns a Report if the session is not found or the LLM call fails.
    async fn handle_llm(
        &self,
        session_id: &str,
        prompt: &str,
        system_prompt: Option<&str>,
    ) -> Result<String, Report<LuaHostHandlerError>> {
        let session_id_typed = crate::protocol::SessionId::from(session_id.to_owned());
        self.ctx
            .send_llm_request_cloned(
                &session_id_typed,
                prompt.to_owned(),
                system_prompt.map(std::borrow::ToOwned::to_owned),
                None,
            )
            .await
            .change_context(LuaHostHandlerError)
            .attach("llm request failed")
    }

    /// Handles a PushUser request.
    ///
    /// Dispatches a [`Command::PushChatEntry`] with a [`ChatEntry::user`] through
    /// the domain node context. The session actor processes the command asynchronously.
    fn handle_push_user(&self, session_id: &str, text: &str) -> Result<(), Report<LuaHostHandlerError>> {
        let session_id_typed = crate::protocol::SessionId::from(session_id.to_owned());
        let entry = ChatEntry::user(text);
        self.ctx.send_command(Command::PushChatEntry(PushChatEntry {
            session_id: session_id_typed,
            entry,
        }));
        Ok(())
    }

    /// Handles a PushSystem request.
    ///
    /// Dispatches a [`Command::PushChatEntry`] with a [`ChatEntry::system`] through
    /// the domain node context. The session actor processes the command asynchronously.
    fn handle_push_system(&self, session_id: &str, text: &str) -> Result<(), Report<LuaHostHandlerError>> {
        let session_id_typed = crate::protocol::SessionId::from(session_id.to_owned());
        let entry = ChatEntry::system(text);
        self.ctx.send_command(Command::PushChatEntry(PushChatEntry {
            session_id: session_id_typed,
            entry,
        }));
        Ok(())
    }

    /// Handles a TurnOff request.
    ///
    /// Finds the attached workflow by ID and sets `enabled = false` and `state = Completed`.
    fn handle_turn_off(&self, workflow_id: &str) -> Result<(), Report<LuaHostHandlerError>> {
        let mut guard = self.state.write();

        // Search the active session for the matching attached workflow.
        let session = guard.session.active_session_mut();
        for aw in &mut session.core.attached_workflows {
            if aw.id.to_string() == workflow_id {
                aw.enabled = false;
                aw.state = AttachedWorkflowState::Completed;
                tracing::info!(workflow = %workflow_id, "lua workflow turned off");
                return Ok(());
            }
        }

        Err(Report::new(LuaHostHandlerError).attach(format!("workflow not found: {workflow_id}")))
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unwrap_in_result,
        reason = "test code, panics are acceptable"
    )]

    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::services::test_services::TestServices;
    use crate::feat::workflow::attached_workflow::WorkflowId;
    use crate::feat::workflow::attached_workflow::{
        AttachedWorkflow, WorkflowConfig, WorkflowTrigger,
    };
    use crate::protocol::{AppMsg, SessionId};
    use tokio::sync::oneshot;

    /// Creates a handler with test services and returns the actor channel receiver
    /// for verifying dispatched commands.
    fn make_handler_with_channel(state: State) -> (LuaHostHandler, kanal::Receiver<AppMsg>) {
        let (tx, rx) = kanal::unbounded::<AppMsg>();
        let services = TestServices::builder().actor_channel(tx).build();
        let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));
        (LuaHostHandler::new(state, ctx), rx)
    }

    /// Creates a handler with test services for tests that don't inspect commands.
    fn make_handler(state: State) -> LuaHostHandler {
        let services = TestServices::builder().build();
        let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));
        LuaHostHandler::new(state, ctx)
    }

    fn make_state_with_session() -> (State, SessionId) {
        let state = State::new(AppState::default());
        let session_id = state.read().session.active_session_id().clone();
        (state, session_id)
    }

    fn make_state_with_session_and_workflow() -> (State, SessionId, WorkflowId) {
        let state = State::new(AppState::default());
        let session_id = state.read().session.active_session_id().clone();

        let workflow_id = WorkflowId::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig {
                script: "judge_fail".to_owned(),
                data: serde_json::json!({}),
            },
            WorkflowTrigger::TurnEnd,
        );
        // Override the ID to our known value.
        {
            let mut guard = state.write();
            let session = guard.session.get_mut(&session_id).expect("session");
            let mut custom_aw = aw;
            custom_aw.id = workflow_id.clone();
            session.core.attached_workflows.push(custom_aw);
        }
        (state, session_id, workflow_id)
    }

    // ── PushUser ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn push_user_dispatches_push_chat_entry_command() {
        // Given a handler with a session and command channel.
        let (state, session_id) = make_state_with_session();
        let (handler, rx) = make_handler_with_channel(state);

        // When handling a PushUser request.
        let (resp_tx, resp_rx) = oneshot::channel();
        handler
            .handle_request(HostRequest::PushUser {
                session_id: session_id.to_string(),
                text: "judgement failed, try again".to_owned(),
                respond_to: resp_tx,
            })
            .await;

        // Then the response is Ok.
        resp_rx.await.expect("response").expect("push_user");

        // And a PushChatEntry command was dispatched with a User entry.
        let msg = rx.try_recv().ok().flatten().expect("command");
        let command = match msg {
            AppMsg::Command { command, .. } => command,
            other => panic!("expected Command, got {other:?}"),
        };
        match command {
            Command::PushChatEntry(pce) => {
                assert_eq!(pce.session_id, session_id);
                match &pce.entry.kind {
                    crate::feat::session::chat_entry::ChatEntryKind::User { display, .. } => {
                        assert_eq!(display, "judgement failed, try again");
                    }
                    other => panic!("expected User entry, got {other:?}"),
                }
            }
            other => panic!("expected PushChatEntry, got {other:?}"),
        }
    }



    // ── PushSystem ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn push_system_dispatches_push_chat_entry_command() {
        // Given a handler with a session and command channel.
        let (state, session_id) = make_state_with_session();
        let (handler, rx) = make_handler_with_channel(state);

        // When handling a PushSystem request.
        let (resp_tx, resp_rx) = oneshot::channel();
        handler
            .handle_request(HostRequest::PushSystem {
                session_id: session_id.to_string(),
                text: "judgement passed".to_owned(),
                respond_to: resp_tx,
            })
            .await;

        // Then the response is Ok.
        resp_rx.await.expect("response").expect("push_system");

        // And a PushChatEntry command was dispatched with a System entry.
        let msg = rx.try_recv().ok().flatten().expect("command");
        let command = match msg {
            AppMsg::Command { command, .. } => command,
            other => panic!("expected Command, got {other:?}"),
        };
        match command {
            Command::PushChatEntry(pce) => {
                assert_eq!(pce.session_id, session_id);
                match &pce.entry.kind {
                    crate::feat::session::chat_entry::ChatEntryKind::System(text) => {
                        assert_eq!(text, "judgement passed");
                    }
                    other => panic!("expected System entry, got {other:?}"),
                }
            }
            other => panic!("expected PushChatEntry, got {other:?}"),
        }
    }

    // ── TurnOff ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn turn_off_disables_attached_workflow() {
        // Given a session with an attached workflow.
        let (state, session_id, workflow_id) = make_state_with_session_and_workflow();
        let handler = make_handler(state.clone());

        // When handling a TurnOff request for that workflow.
        let (resp_tx, resp_rx) = oneshot::channel();
        handler
            .handle_request(HostRequest::TurnOff {
                workflow_id: workflow_id.to_string(),
                respond_to: resp_tx,
            })
            .await;

        // Then the response is Ok.
        resp_rx.await.expect("response").expect("turn_off");

        // And the workflow is disabled and Completed.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        let aw = session
            .core
            .attached_workflows
            .iter()
            .find(|aw| aw.id == workflow_id)
            .expect("workflow");
        assert!(!aw.enabled);
        assert!(matches!(aw.state, AttachedWorkflowState::Completed));
    }

    #[tokio::test]
    async fn turn_off_returns_error_for_unknown_workflow() {
        // Given a session with an attached workflow.
        let (state, _, _) = make_state_with_session_and_workflow();
        let handler = make_handler(state);

        // When handling a TurnOff request for a nonexistent workflow.
        let (resp_tx, resp_rx) = oneshot::channel();
        handler
            .handle_request(HostRequest::TurnOff {
                workflow_id: "nonexistent".to_owned(),
                respond_to: resp_tx,
            })
            .await;

        // Then the response is an error.
        let result = resp_rx.await.expect("response");
        assert!(result.is_err());
    }

    // ── Run loop ──────────────────────���────────────────────────────────

    #[tokio::test]
    async fn run_loop_processes_requests_until_shutdown() {
        // Given a handler with a session and command channel.
        let (state, session_id) = make_state_with_session();
        let (handler, rx) = make_handler_with_channel(state);

        // When sending a PushUser then a Shutdown through the run loop.
        let (tx, host_rx) = kanal::unbounded::<HostRequest>();
        let (resp_tx, resp_rx) = oneshot::channel();
        tx.send(HostRequest::PushUser {
            session_id: session_id.to_string(),
            text: "from loop".to_owned(),
            respond_to: resp_tx,
        })
        .expect("send");
        tx.send(HostRequest::Shutdown).expect("send");

        handler.run(host_rx).await;

        // Then the response is Ok.
        resp_rx.await.expect("response").expect("push_user");

        // And a PushChatEntry command was dispatched.
        let msg = rx.try_recv().ok().flatten().expect("command");
        let command = match msg {
            AppMsg::Command { command, .. } => command,
            other => panic!("expected Command, got {other:?}"),
        };
        match command {
            Command::PushChatEntry(pce) => {
                match &pce.entry.kind {
                    crate::feat::session::chat_entry::ChatEntryKind::User { display, .. } => {
                        assert_eq!(display, "from loop");
                    }
                    other => panic!("expected User entry, got {other:?}"),
                }
            }
            other => panic!("expected PushChatEntry, got {other:?}"),
        }
    }
}
