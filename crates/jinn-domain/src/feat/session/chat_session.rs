//! Compatibility exports for the authoritative live session aggregate.

pub use jinn_chat_log_view_msg::SavedHistoryPosition;
pub use jinn_session_lifecycle_msg::LifecycleScriptState;
pub use jinn_session_msg::SessionOrigin;
pub use jinn_session_state::{
    ChatSessionState, SessionCore, SessionCoreEphemeral, SessionProfile, SessionUi, StreamingError,
    default_persist,
};
pub use jinn_session_store_msg::SessionState;
