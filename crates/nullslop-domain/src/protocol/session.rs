//! Session identity types and persistence events.
//!
//! Re-exports from `feat::session::protocol`.

pub use crate::feat::session::protocol::session_id::SessionId;
pub use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
pub use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;
pub use crate::feat::session::protocol::session_new::SessionNew;
pub use crate::feat::session::protocol::session_save_requested::SessionSaveRequested;
