//! Session protocol — session identity and lifecycle types.

pub mod archive_session;
pub mod close_session;
pub mod history_appended;
pub mod load_session_picker_entries;
pub mod mark_session_interacted;
pub mod session_archived;
pub mod session_closed;
pub mod session_fork_requested;
pub mod session_id;
pub mod session_load_completed;
pub mod session_load_requested;
pub mod session_new;
pub mod session_phase_changed;
pub mod submit_history_mutations;
pub mod user_interacted;

pub use archive_session::ArchiveSession;
pub use close_session::CloseSession;
pub use mark_session_interacted::MarkSessionInteracted;
pub use session_archived::SessionArchived;
pub use session_closed::SessionClosed;
pub use submit_history_mutations::SubmitHistoryMutations;
pub use user_interacted::UserInteracted;
