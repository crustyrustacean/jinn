//! Event handlers — process streaming and tool call events.

use nullslop_protocol::context::PromptAssembled;
use nullslop_protocol::provider::{SendToLlmProvider, StreamCompleted, StreamToken};
use nullslop_protocol::tool::{
    ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted, ToolUseStarted,
};
use nullslop_protocol::{ChatEntry, Command};

use super::super::SessionPersistenceActor;

impl SessionPersistenceActor {
    /// PromptAssembled (event): transition session from assembling to streaming,
    /// emit SendToLlmProvider.
    pub(in crate::session::actor) fn handle_prompt_assembled(
        &self,
        payload: &PromptAssembled,
        ctx: &nullslop_actor::ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            if session.is_assembling() {
                session.finish_assembling();
            }
            if session.is_sending() {
                session.finish_sending();
            }
            session.begin_streaming();
        }

        if let Err(e) = ctx.send_command(Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: payload.session_id.clone(),
                messages: payload.messages.clone(),
                provider_id: None,
            },
        }) {
            tracing::warn!(err = ?e, "session-actor failed to emit SendToLlmProvider");
        }
    }

    /// Appends a streaming token to the session's assistant entry.
    pub(in crate::session::actor) fn on_stream_token(&self, event: &StreamToken) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        if !session.is_streaming() {
            session.begin_streaming();
        }
        session.append_stream_token(&event.token);
    }

    /// Marks the session's stream as finished.
    pub(in crate::session::actor) fn on_stream_completed(&self, event: &StreamCompleted) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.finish_streaming();
    }

    /// Begins tracking a streaming tool call.
    pub(in crate::session::actor) fn on_tool_use_started(&self, event: &ToolUseStarted) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.begin_tool_call(event.index, &event.id, &event.name);
    }

    /// Pushes a tool call entry into the session history.
    pub(in crate::session::actor) fn on_tool_call_received(&self, event: &ToolCallReceived) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.push_entry(ChatEntry::tool_call(
            &event.tool_call.id,
            &event.tool_call.name,
            &event.tool_call.arguments,
        ));
    }

    /// Appends a partial JSON delta to a streaming tool call.
    pub(in crate::session::actor) fn on_tool_call_streaming(&self, event: &ToolCallStreaming) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.append_tool_call_delta(event.index, &event.partial_json);
    }

    /// Pushes a tool result entry into the session history.
    pub(in crate::session::actor) fn on_tool_execution_completed(
        &self,
        event: &ToolExecutionCompleted,
    ) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.push_entry(ChatEntry::tool_result(
            &event.result.tool_call_id,
            &event.result.name,
            &event.result.content,
            event.result.success,
        ));
    }
}
