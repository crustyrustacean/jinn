//! Domain-specific implementation of [`NodeContext`].
//!
//! [`DomainNodeContext`] bridges the workflow engine to the domain layer,
//! providing LLM access through the existing actor bus infrastructure.
//! Each LLM node call creates a new session, sends a `SendToLlmProvider`
//! command, and awaits the response via a oneshot channel.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use error_stack::Report;
use nullslop_provider::LlmMessage;
use nullslop_workflow::NodeStatus;
use nullslop_workflow::node::{NodeContext, NodeError};
use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::provider::protocol::command::SendToLlmProvider;
use crate::feat::session::chat_session::ChatSessionState;
use crate::protocol::{Command, SessionId};

/// Domain-specific implementation of [`NodeContext`].
///
/// Provides:
/// - `send_llm_request` — sends a prompt to the LLM and returns the full response
pub struct DomainNodeContext {
    /// Shared services for accessing the actor bus.
    services: Services,
    /// Shared application state.
    state: State,
    /// Maps session IDs to pending oneshot senders.
    /// When `WorkflowActor` receives `StreamCompleted`, it resolves the matching sender.
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

    /// Send an LLM request and wait for the full response.
    ///
    /// Creates a new session with `is_workflow: true`, emits `SendToLlmProvider`,
    /// and awaits the response via a oneshot channel.
    ///
    /// # Errors
    ///
    /// Returns an error if the LLM request fails or times out.
    #[expect(clippy::missing_errors_doc, reason = "error variant is generic NodeError")]
    pub async fn send_llm_request_inner(
        &self,
        system_prompt: String,
        user_prompt: String,
        provider_id: Option<String>,
    ) -> Result<String, Report<NodeError>> {
        let session_id = SessionId::new();

        // Create a new session with is_workflow: true.
        let mut session = ChatSessionState::new();
        session.core.is_workflow = true;

        // Insert into app state.
        {
            let mut state = self.state.write();
            state.session.insert(session);
            state.session.set_active(session_id.clone());
        }

        // Build messages.
        let messages = vec![
            LlmMessage::System {
                content: system_prompt,
            },
            LlmMessage::User {
                content: user_prompt,
            },
        ];

        // Create oneshot channel.
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock();
            pending.insert(session_id.clone(), tx);
        }

        // Send the command.
        self.services
            .actor_channel
            .send_command(Command::SendToLlmProvider(SendToLlmProvider {
                session_id: session_id.clone(),
                messages,
                tool_definitions: vec![],
                provider_id,
                estimated_tokens: 0,
            }));

        // Await the response.
        rx.await
            .map_err(|_| Report::new(NodeError).attach("workflow LLM request cancelled"))
    }

    /// Called by `WorkflowActor` when `StreamCompleted` arrives.
    /// Resolves the oneshot for the given session ID.
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
        system_prompt: &str,
        user_prompt: &str,
        provider_id: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<String, Report<NodeError>>> + Send + 'a>> {
        Box::pin(self.send_llm_request_inner(
            system_prompt.to_owned(),
            user_prompt.to_owned(),
            provider_id.map(std::borrow::ToOwned::to_owned),
        ))
    }
}
