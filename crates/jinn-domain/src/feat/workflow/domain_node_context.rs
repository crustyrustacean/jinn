//! Domain-specific implementation of [`NodeContext`].
//!
//! [`DomainNodeContext`] bridges the workflow engine to the domain layer,
//! providing LLM access through the existing session infrastructure.
//! Each LLM node call creates a new workflow session, stores assembly overrides,
//! enqueues a user message, and awaits the response via a oneshot channel.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use error_stack::Report;
use jinn_workflow::node::{NodeContext, NodeError};
use jinn_workflow::tool_schema::ToolSchema;
use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::EnqueueUserMessage;
use crate::feat::context::assemble::AssemblyOverrides;
use crate::feat::session::chat_entry::ChatEntry;
use crate::feat::session::chat_session::{ChatSessionState, SessionCoreEphemeral};
use crate::feat::workflow::tool_mapping::tool_schemas_to_definitions;
use crate::protocol::{Command, SessionId};

/// Domain-specific implementation of [`NodeContext`].
///
/// Provides:
/// - `send_llm_request` - creates a workflow session, enqueues a user message,
///   and returns the full response once the session returns to `Idle`.
pub struct DomainNodeContext {
    /// Shared services for accessing the actor bus.
    services: Services,
    /// Shared application state.
    state: State,
    /// Maps session IDs to pending oneshot senders.
    /// When `WorkflowActor` receives `SessionPhaseChanged(Idle)` for a workflow
    /// session, it resolves the matching sender with the last assistant message.
    pending: Arc<Mutex<HashMap<SessionId, oneshot::Sender<String>>>>,
    /// Current node being executed (set by engine via `set_node_name`).
    current_node_name: Arc<Mutex<Option<String>>>,
    /// Current attached workflow ID being executed (set by controller before spawning).
    /// Used to register node→session mappings in `state.workflow_executions`.
    current_workflow_id: Arc<Mutex<Option<crate::feat::workflow::workflow_state::WorkflowId>>>,
}

impl DomainNodeContext {
    /// Create a new domain node context.
    pub fn new(services: Services, state: State) -> Self {
        Self {
            services,
            state,
            pending: Arc::new(Mutex::new(HashMap::new())),
            current_node_name: Arc::new(Mutex::new(None)),
            current_workflow_id: Arc::new(Mutex::new(None)),
        }
    }

    /// Send an LLM request through the session pipeline and wait for the full response.
    ///
    /// Creates a new session with `is_workflow: true`, stores `AssemblyOverrides`
    /// derived from the provided parameters, enqueues a user message, and awaits
    /// the response via a oneshot channel.
    ///
    /// # Errors
    ///
    /// Returns an error if the LLM request fails or times out.
    pub async fn send_llm_request_inner(
        &self,
        user_prompt: String,
        system_prompt: Option<String>,
        tool_schemas: Vec<ToolSchema>,
        provider_id: Option<String>,
    ) -> Result<String, Report<NodeError>> {
        // Build assembly overrides from the node's configuration.
        let tool_definitions = if tool_schemas.is_empty() {
            None
        } else {
            Some(tool_schemas_to_definitions(&tool_schemas))
        };

        let overrides = AssemblyOverrides {
            system_prompt,
            tool_definitions,
            skip_skills: true,
            skip_context_files: true,
        };

        // Create a new session with is_workflow: true and store overrides.
        let mut session = ChatSessionState::new();
        session.core.is_workflow = true;
        session.core.workflow_overrides = Some(overrides);

        // Resolve provider: explicit node config takes priority, otherwise
        // inherit the active session's model so the LLM actor can create
        // a per-request factory via the provider registry.
        let model = provider_id.unwrap_or_else(|| {
            let state = self.state.read();
            state.active_session().profile().model.clone()
        });
        session.set_model(model);

        let session_id = session.session_id().clone();

        // Insert into app state.
        {
            let mut state = self.state.write();
            state.session.insert(session);
            state.session.set_active(session_id.clone());
        }

        // Record node→session mapping in active workflow state.
        let node_name = self.current_node_name.lock().clone();
        if let Some(name) = node_name {
            let mut state = self.state.write();
            if let Some(workflow) = state.workflow.active_mut() {
                workflow.node_sessions.insert(name, session_id.clone());
            }
        }

        // Create oneshot channel for the response.
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock();
            pending.insert(session_id.clone(), tx);
        }

        // Enqueue the user message via the session command pipeline.
        let entry = ChatEntry::user(&user_prompt);
        self.services
            .actor_channel
            .send_command(Command::EnqueueUserMessage(EnqueueUserMessage {
                session_id: session_id.clone(),
                entry,
            }));

        // Await the response.
        rx.await
            .map_err(|_| Report::new(NodeError).attach("workflow LLM request cancelled"))
    }

    /// Returns `true` if there is a pending oneshot for the given session ID.
    pub fn has_pending(&self, session_id: &SessionId) -> bool {
        let pending = self.pending.lock();
        pending.contains_key(session_id)
    }

    /// Called by `WorkflowActor` when `SessionPhaseChanged(Idle)` arrives
    /// for a workflow session. Reads the last assistant message from history
    /// and resolves the pending oneshot.
    pub fn resolve_completed(&self, session_id: &SessionId, response: String) {
        let mut pending = self.pending.lock();
        if let Some(tx) = pending.remove(session_id) {
            let _ = tx.send(response);
        }
    }

    /// Inserts a pending oneshot sender for the given session ID.
    ///
    /// Test-only helper for setting up workflow resolution scenarios.
    #[cfg(test)]
    pub fn insert_pending(&self, session_id: SessionId, tx: oneshot::Sender<String>) {
        self.pending.lock().insert(session_id, tx);
    }

    /// Set the current attached workflow ID (called by controller before execution).
    pub fn set_workflow_id(&self, id: crate::feat::workflow::workflow_state::WorkflowId) {
        *self.current_workflow_id.lock() = Some(id);
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
        tool_schemas: Vec<ToolSchema>,
        provider_id: Option<String>,
    ) -> Result<String, Report<NodeError>> {
        // 1. Read source session, clone it entirely
        let mut session = {
            let guard = self.state.read();
            guard
                .session
                .get(source_session_id)
                .cloned()
                .ok_or_else(|| Report::new(NodeError).attach("source session not found"))?
        };

        // 2. Build overrides
        let overrides = AssemblyOverrides {
            system_prompt,
            tool_definitions: if tool_schemas.is_empty() {
                None
            } else {
                Some(tool_schemas_to_definitions(&tool_schemas))
            },
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

        // 7. Record node→session mapping for the running attached workflow
        let node_name = self.current_node_name.lock().clone();
        if let (Some(name), Some(wid)) = (node_name, self.current_workflow_id.lock().clone()) {
            let mut state = self.state.write();
            if let Some(exec_state) = state.workflow_executions.get_mut(&wid) {
                exec_state.node_sessions.insert(name, session_id.clone());
            }
        }

        // 8. Create oneshot, enqueue, await (same pattern as existing)
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(session_id.clone(), tx);

        let entry = ChatEntry::user(&user_prompt);
        self.services.actor_channel.send_command(
            Command::EnqueueUserMessage(EnqueueUserMessage { session_id: session_id.clone(), entry }),
        );

        rx.await
            .map_err(|_| Report::new(NodeError).attach("cloned workflow LLM request cancelled"))
    }
}




impl NodeContext for DomainNodeContext {
    fn set_node_name(&self, name: &str) {
        *self.current_node_name.lock() = Some(name.to_owned());
    }

    fn clear_node_name(&self) {
        *self.current_node_name.lock() = None;
    }

    fn send_llm_request<'a>(
        &'a self,
        user_prompt: &str,
        system_prompt: Option<&str>,
        tool_schemas: Vec<ToolSchema>,
        provider_id: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<String, Report<NodeError>>> + Send + 'a>> {
        Box::pin(self.send_llm_request_inner(
            user_prompt.to_owned(),
            system_prompt.map(std::borrow::ToOwned::to_owned),
            tool_schemas,
            provider_id.map(std::borrow::ToOwned::to_owned),
        ))
    }

    fn clone_arc(&self) -> Option<Arc<dyn jinn_workflow::node::NodeContext>> {
        Some(Arc::new(Self {
            services: self.services.clone(),
            state: self.state.clone(),
            pending: Arc::clone(&self.pending),
            current_node_name: Arc::clone(&self.current_node_name),
            current_workflow_id: Arc::clone(&self.current_workflow_id),
        }))
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
        // Given a new context.
        let ctx = make_ctx();

        // Then has_pending returns false for any session.
        assert!(!ctx.has_pending(&SessionId::new()));
    }

    #[rstest::rstest]
    fn has_pending_returns_true_after_resolve_setup() {
        // Given a context and a pending session.
        let ctx = make_ctx();
        let session_id = SessionId::new();

        // When we manually insert a pending entry.
        let (tx, rx) = oneshot::channel();
        ctx.pending.lock().insert(session_id.clone(), tx);

        // Then has_pending returns true.
        assert!(ctx.has_pending(&session_id));

        // And the receiver is still pending.
        // Drop tx without sending to avoid leak.
        drop(rx);
    }

    #[rstest::rstest]
    fn resolve_completed_sends_response() {
        // Given a context with a pending oneshot.
        let ctx = make_ctx();
        let session_id = SessionId::new();
        let (tx, mut rx) = oneshot::channel();
        ctx.pending.lock().insert(session_id.clone(), tx);

        // When resolving with a response.
        ctx.resolve_completed(&session_id, "hello world".to_owned());

        // Then the receiver gets the response.
        let result = rx.try_recv().expect("should have a value");
        assert_eq!(result, "hello world");
    }

    #[rstest::rstest]
    fn resolve_completed_removes_pending() {
        // Given a context with a pending oneshot.
        let ctx = make_ctx();
        let session_id = SessionId::new();
        let (tx, _rx) = oneshot::channel();
        ctx.pending.lock().insert(session_id.clone(), tx);

        // When resolving.
        ctx.resolve_completed(&session_id, "response".to_owned());

        // Then has_pending returns false.
        assert!(!ctx.has_pending(&session_id));
    }

    #[rstest::rstest]
    fn resolve_completed_ignores_unknown_session() {
        // Given a context with no pending entries.
        let ctx = make_ctx();
        let session_id = SessionId::new();

        // When resolving an unknown session.
        // Then it does not panic.
        ctx.resolve_completed(&session_id, "response".to_owned());
    }

    #[rstest::rstest]
    fn set_node_name_stores_name() {
        // Given a new context.
        let ctx = make_ctx();

        // When setting the node name.
        ctx.set_node_name("my-node");

        // Then it is stored.
        assert_eq!(*ctx.current_node_name.lock(), Some("my-node".to_owned()));
    }

    #[rstest::rstest]
    fn clear_node_name_clears() {
        // Given a context with a node name set.
        let ctx = make_ctx();
        ctx.set_node_name("my-node");

        // When clearing.
        ctx.clear_node_name();

        // Then it is None.
        assert_eq!(*ctx.current_node_name.lock(), None);
    }

    #[rstest::rstest]
    fn clear_node_name_when_already_none_is_noop() {
        // Given a new context (name is None).
        let ctx = make_ctx();

        // When clearing.
        ctx.clear_node_name();

        // Then it is still None.
        assert_eq!(*ctx.current_node_name.lock(), None);
    }
}
