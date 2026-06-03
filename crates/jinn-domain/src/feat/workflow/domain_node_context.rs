//! Domain context for Lua workflow LLM access.
//!
//! Provides `send_llm_request_cloned` so Lua scripts can call `ctx.llm()`
//! through the existing session infrastructure. Also provides `send_command`
//! for the controller to emit domain commands.

use std::collections::HashMap;
use std::sync::Arc;

use error_stack::Report;
use parking_lot::Mutex;
use tokio::sync::oneshot;
use wherror::Error;

use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::EnqueueUserMessage;
use crate::feat::context::assemble::AssemblyOverrides;
use crate::feat::session::chat_entry::ChatEntry;
use crate::feat::session::chat_session::{ChatSessionState, SessionCoreEphemeral};
use crate::protocol::{Command, SessionId};

/// Error for domain context operations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct DomainContextError;

/// Domain context for Lua workflow LLM access.
///
/// Provides:
/// - `send_command` - emit domain commands through the actor channel
/// - `send_llm_request_cloned` - clone a session and send an LLM request
pub struct DomainNodeContext {
    /// Shared services for accessing the actor bus.
    services: Services,
    /// Shared application state.
    state: State,
    /// Maps session IDs to pending oneshot senders.
    pending: Arc<Mutex<HashMap<SessionId, oneshot::Sender<String>>>>,
}

impl DomainNodeContext {
    /// Create a new domain context.
    pub fn new(services: Services, state: State) -> Self {
        Self {
            services,
            state,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Send a command through the actor channel.
    pub fn send_command(&self, cmd: Command) {
        self.services.actor_channel.send_command(cmd);
    }

    /// Returns `true` if there is a pending oneshot for the given session ID.
    pub fn has_pending(&self, session_id: &SessionId) -> bool {
        self.pending.lock().contains_key(session_id)
    }

    /// Resolves a pending oneshot with the given response.
    pub fn resolve_completed(&self, session_id: &SessionId, response: String) {
        if let Some(tx) = self.pending.lock().remove(session_id) {
            let _ = tx.send(response);
        }
    }

    /// Inserts a pending oneshot sender for the given session ID.
    #[cfg(test)]
    pub fn insert_pending(&self, session_id: SessionId, tx: oneshot::Sender<String>) {
        self.pending.lock().insert(session_id, tx);
    }

    /// Send an LLM request using a cloned session and wait for the full response.
    ///
    /// Clones an existing session, giving the clone a new ID, `is_workflow = true`,
    /// and `parent_session = Some(source)`. The clone inherits full history, profile,
    /// and tools from the source.
    ///
    /// # Errors
    ///
    /// Returns an error if the source session is not found or the oneshot is cancelled.
    pub async fn send_llm_request_cloned(
        &self,
        source_session_id: &SessionId,
        user_prompt: String,
        system_prompt: Option<String>,
        provider_id: Option<String>,
    ) -> Result<String, Report<DomainContextError>> {
        // 1. Read source session, clone it entirely
        let mut session = {
            let guard = self.state.read();
            guard
                .session
                .get(source_session_id)
                .cloned()
                .ok_or_else(|| Report::new(DomainContextError).attach("source session not found"))?
        };

        // 2. Build overrides
        let overrides = AssemblyOverrides {
            system_prompt,
            tool_definitions: None,
            skip_skills: true,
            skip_context_files: true,
        };

        // 3. Generate new session ID (clone must NOT share ID with source)
        session.core.session_id = SessionId::new();

        // 4. Mark as workflow, reset ephemeral
        session.core.is_workflow = true;
        session.core.ephemeral = SessionCoreEphemeral::default();
        session.core.workflow_overrides = Some(overrides);
        session.core.parent_session = Some(source_session_id.clone());

        // 5. Resolve model
        let model = provider_id.unwrap_or_else(|| session.core.profile.model.clone());
        session.set_model(model);

        let session_id = session.session_id().clone();

        // 6. Insert into app state
        {
            let mut state = self.state.write();
            state.session.insert(session);
            state.session.set_active(session_id.clone());
        }

        // 7. Create oneshot, enqueue, await
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(session_id.clone(), tx);

        let entry = ChatEntry::user(&user_prompt);
        self.services
            .actor_channel
            .send_command(Command::EnqueueUserMessage(EnqueueUserMessage {
                session_id: session_id.clone(),
                entry,
            }));

        rx.await.map_err(|_| {
            Report::new(DomainContextError).attach("cloned workflow LLM request cancelled")
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::services::test_services::TestServices;
    use crate::common::state::State;

    fn make_ctx() -> DomainNodeContext {
        let services = TestServices::builder().build();
        let state = State::new(AppState::default());
        DomainNodeContext::new(services, state)
    }

    #[rstest::rstest]
    fn has_pending_returns_false_when_empty() {
        let ctx = make_ctx();
        assert!(!ctx.has_pending(&SessionId::new()));
    }

    #[rstest::rstest]
    fn has_pending_returns_true_after_insert() {
        let ctx = make_ctx();
        let session_id = SessionId::new();
        let (tx, rx) = oneshot::channel();
        ctx.pending.lock().insert(session_id.clone(), tx);
        assert!(ctx.has_pending(&session_id));
        drop(rx);
    }

    #[rstest::rstest]
    fn resolve_completed_sends_response() {
        let ctx = make_ctx();
        let session_id = SessionId::new();
        let (tx, mut rx) = oneshot::channel();
        ctx.pending.lock().insert(session_id.clone(), tx);
        ctx.resolve_completed(&session_id, "hello world".to_owned());
        let result = rx.try_recv().expect("should have a value");
        assert_eq!(result, "hello world");
    }

    #[rstest::rstest]
    fn resolve_completed_removes_pending() {
        let ctx = make_ctx();
        let session_id = SessionId::new();
        let (tx, _rx) = oneshot::channel();
        ctx.pending.lock().insert(session_id.clone(), tx);
        ctx.resolve_completed(&session_id, "response".to_owned());
        assert!(!ctx.has_pending(&session_id));
    }

    #[rstest::rstest]
    fn resolve_completed_ignores_unknown_session() {
        let ctx = make_ctx();
        ctx.resolve_completed(&SessionId::new(), "response".to_owned());
    }
}
