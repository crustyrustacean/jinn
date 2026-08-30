//! Events the coordinator actor publishes.

use serde::{Deserialize, Serialize};

use super::command::TermSessionId;

/// The active session's screen mirror changed.
///
/// Published on every parsed output batch while a settle wait is running
/// (throttled to screen changes) so the takeover view and any open tool-call
/// entry stay current. Doubles as the stall watchdog's keepalive: the
/// session actor appends these to the pending tool result, bumping history
/// activity so a long interactive call is never falsely retried.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermScreenUpdated {
    /// The session whose screen changed.
    pub session_id: TermSessionId,
    /// The rendered screen (plain text).
    pub screen: String,
    /// The styled cell grid matching `screen`.
    pub cells: crate::feat::interactive_term::emulator::ScreenCells,
    /// Cursor position as (row, col).
    pub cursor: (u16, u16),
    /// Whether the program hid the cursor.
    pub cursor_hidden: bool,
}

/// The user took (or released) control of the active session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermControlChanged {
    /// The session control flipped for.
    pub session_id: TermSessionId,
    /// `true` when the user holds control; `false` when handed back.
    pub user_controls: bool,
}

impl crate::common::bus::BusMessage for TermScreenUpdated {}
impl crate::common::bus::BusMessage for TermControlChanged {}
