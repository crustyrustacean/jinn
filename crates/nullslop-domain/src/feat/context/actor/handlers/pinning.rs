//! Pinning handlers — pin and unpin chat entries.

use crate::feat::context::protocol::command::{PinChatEntry, UnpinChatEntry};

use super::super::PromptAssemblyActor;

impl PromptAssemblyActor {
    /// PinChatEntry: pin entry in session.
    pub(in crate::feat::context::actor) fn handle_pin_chat_entry(&self, payload: &PinChatEntry) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&payload.session_id);
        session.pin_entry(&payload.entry_id, payload.position);
    }

    /// UnpinChatEntry: unpin entry in session.
    pub(in crate::feat::context::actor) fn handle_unpin_chat_entry(
        &self,
        payload: &UnpinChatEntry,
    ) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&payload.session_id);
        session.unpin_entry(&payload.entry_id);
    }
}
