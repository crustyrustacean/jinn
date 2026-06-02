//! Channel protocol between Lua VM tasks and the host.
//!
//! Each VM capability call sends a [`HostRequest`] through the channel.
//! The host handler processes the request and responds via the oneshot
//! channel included in each variant.

use std::fmt;

use tokio::sync::oneshot;

/// Workflow ID type — opaque string identifier.
pub type WorkflowId = String;

/// Session ID type — opaque string identifier.
pub type SessionId = String;

/// Requests sent from Lua VM tasks to the host.
///
/// Each variant that needs a response carries a [`oneshot::Sender`].
/// The host handler processes the request and sends the result through it.
#[derive(Debug)]
pub enum HostRequest {
    /// Call the LLM with a prompt and return the response text.
    Llm {
        /// The session to run the LLM call in.
        session_id: SessionId,
        /// The user prompt to send.
        prompt: String,
        /// Optional system prompt override.
        system_prompt: Option<String>,
        /// Channel to send the response back.
        respond_to: oneshot::Sender<Result<String, String>>,
    },
    /// Push a user message into session history.
    PushUser {
        /// The session to push the entry into.
        session_id: SessionId,
        /// The message text.
        text: String,
        /// Channel to send confirmation back.
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    /// Push a system message into session history.
    PushSystem {
        /// The session to push the entry into.
        session_id: SessionId,
        /// The message text.
        text: String,
        /// Channel to send confirmation back.
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    /// Turn off (soft-disable) an attached workflow.
    TurnOff {
        /// The workflow to disable.
        workflow_id: WorkflowId,
        /// Channel to send confirmation back.
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    /// Shut down the VM task.
    Shutdown,
}

impl fmt::Display for HostRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Llm { prompt, .. } => write!(f, "Llm({prompt:?})"),
            Self::PushUser { text, .. } => write!(f, "PushUser({text:?})"),
            Self::PushSystem { text, .. } => write!(f, "PushSystem({text:?})"),
            Self::TurnOff { workflow_id, .. } => write!(f, "TurnOff({workflow_id})"),
            Self::Shutdown => write!(f, "Shutdown"),
        }
    }
}
