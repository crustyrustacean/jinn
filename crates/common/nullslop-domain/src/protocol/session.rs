//! Session identity types and persistence events.
//!
//! A [`SessionId`] uniquely identifies a chat session. It is generated
//! using UUID v4 and stored as an opaque string.
//!
//! [`SessionSaveRequested`] is emitted by the message queue handler to
//! trigger asynchronous session persistence via the actor system.
//! [`SessionLoadRequested`] is emitted when the user picks a session from
//! the session browser. [`SessionLoadCompleted`] carries the loaded data back.
//! [`SessionNew`] closes the session picker and starts a fresh session.

mod session_id;
pub mod session_load_completed;
pub mod session_load_requested;
pub mod session_new;
pub mod session_save_requested;

pub use session_id::SessionId;
pub use session_load_completed::SessionLoadCompleted;
pub use session_load_requested::SessionLoadRequested;
pub use session_new::SessionNew;
pub use session_save_requested::SessionSaveRequested;
