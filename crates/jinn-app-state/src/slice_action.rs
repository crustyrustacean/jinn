//! [`AppState`]'s implementation of [`SliceActionState`].
//!
//! The route table and vocabulary live in [`jinn_slices::route`]. This module
//! holds the one thing that cannot live there — an implementation of a
//! `jinn-slices` trait for a type defined in *this* crate. The orphan rule
//! requires the impl to sit with the type, which is why it moved out of the
//! kernel along with [`AppState`] itself.

use std::path::PathBuf;

use jinn_core_types::ChatEntry;
use jinn_core_types::SessionId;
use jinn_slices::PublishClosure;
use jinn_slices::PublishSink;
use jinn_slices::PublishableMessage;
use jinn_slices::route::SliceActionState;

use crate::app_state::AppState;

impl SliceActionState for AppState {
    fn active_session_title(&self) -> Option<String> {
        self.active_session().title().map(str::to_owned)
    }

    fn active_session_id(&self) -> SessionId {
        self.session.active_session_id().clone()
    }

    fn push_session_error(&mut self, message: &str) {
        self.active_session_mut()
            .push_entry(ChatEntry::error(message));
    }

    fn active_session_cwd(&self) -> PathBuf {
        self.active_session().cwd().to_owned()
    }

    fn publish_session_cwd(&self, session_id: SessionId, cwd: PathBuf) -> PublishClosure {
        publish_message(jinn_session_lifecycle_msg::SetSessionCwd { session_id, cwd })
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// Mints the publish closure for a message destined for the fabric.
///
/// The cwd slice is kernel-free and holds only the opaque closure, so the
/// command type it eventually publishes is named by its caller rather than
/// by the slice.
pub fn publish_message<M>(msg: M) -> PublishClosure
where
    M: PublishableMessage,
{
    Box::new(move |sink: &dyn PublishSink| {
        let payload = serde_json::to_value(&msg).unwrap_or(serde_json::Value::Null);
        sink.publish_schema(M::schema_id(), payload, std::any::type_name::<M>());
    })
}
