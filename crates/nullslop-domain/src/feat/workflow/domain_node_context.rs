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
use nullslop_workflow::node::{NodeContext, NodeError};
use nullslop_workflow::tool_schema::ToolSchema;
use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::EnqueueUserMessage;
use crate::feat::context::assemble::AssemblyOverrides;
use crate::feat::session::chat_entry::ChatEntry;
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::workflow::tool_mapping::tool_schemas_to_definitions;
use crate::protocol::{Command, SessionId};

/// Domain-specific implementation of [`NodeContext`].
///
/// Provides:
/// - `send_llm_request` — creates a workflow session, enqueues a user message,
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
}

impl DomainNodeContext {
    /// Create a new domain node context.
    pub fn new(services: Services, state: State) -> Self {
        Self {
            services,
            state,
            pending: Arc::new(Mutex::new(HashMap::new())),
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
        _provider_id: Option<String>,
    ) -> Result<String, Report<NodeError>> {
        let session_id = SessionId::new();

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

        // Insert into app state.
        {
            let mut state = self.state.write();
            state.session.insert(session);
            state.session.set_active(session_id.clone());
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
}

impl NodeContext for DomainNodeContext {
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
}
