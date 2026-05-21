// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! History appended event — emitted when a new entry is added to session history.
//!
//! The compaction actor subscribes to this event to evaluate whether
//! auto-compaction should be triggered. Unlike `StreamCompleted`, which
//! only fires at turn boundaries, `HistoryAppended` fires for every
//! history entry (user message, assistant message, tool call, tool result),
//! enabling mid-turn compaction triggers during agentic tool loops.

use serde::{Deserialize, Serialize};

use crate::protocol::{EventMsg, SessionId};

/// Emitted when a new entry is appended to the session history.
///
/// Carries the estimated total token count of the full history after
/// the entry was appended, so the compaction actor can evaluate the
/// threshold without re-scanning the entire history.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("session")]
pub struct HistoryAppended {
    /// The session whose history was appended to.
    pub session_id: SessionId,
    /// Estimated token count of the full history after this entry was appended.
    pub total_estimated_tokens: usize,
}
