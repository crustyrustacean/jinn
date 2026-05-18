//! Session protocol — session identity and lifecycle types.

pub mod load_session_picker_entries;
pub mod remove_session;
pub mod session_fork_requested;
pub mod session_id;
pub mod session_load_completed;
pub mod session_load_requested;
pub mod session_new;
pub mod session_removed;

pub use remove_session::RemoveSession;
pub use session_removed::SessionRemoved;
