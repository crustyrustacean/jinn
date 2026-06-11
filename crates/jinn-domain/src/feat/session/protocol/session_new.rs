//! Signal to close the session picker and start a fresh empty session.
//!
//! Bound to CTRL+N in the Picker scope. The handler creates a new
//! [`SessionId`], inserts a fresh `ChatSessionState`, and closes the picker.
//!
//! [`SessionId`]: crate::feat::session::SessionId

use serde::{Deserialize, Serialize};


/// Signal to close the session picker and start a fresh empty session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionNew;
