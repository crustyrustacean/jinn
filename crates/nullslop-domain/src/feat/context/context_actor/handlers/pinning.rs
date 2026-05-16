//! Pinning handlers — pin and unpin chat entries.

use crate::common::actor::ActorContext;
use crate::feat::context::protocol::command::{PinChatEntry, UnpinChatEntry};
use crate::feat::context::protocol::event::ChatEntryPinChanged;
use crate::protocol::{Command, Event};

use super::super::PromptAssemblyActor;

impl PromptAssemblyActor {
    /// PinChatEntry: pin entry in session.
    pub(in crate::feat::context::context_actor) fn handle_pin_chat_entry(
        &self,
        payload: &PinChatEntry,
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.pin_entry(&payload.entry_id, payload.position);
        }
        let _ = ctx.send_event(Event::ChatEntryPinChanged(ChatEntryPinChanged {
            session_id: payload.session_id.clone(),
        }));
    }

    /// UnpinChatEntry: unpin entry in session.
    pub(in crate::feat::context::context_actor) fn handle_unpin_chat_entry(
        &self,
        payload: &UnpinChatEntry,
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.unpin_entry(&payload.entry_id);
        }
        let _ = ctx.send_event(Event::ChatEntryPinChanged(ChatEntryPinChanged {
            session_id: payload.session_id.clone(),
        }));
    }
}
