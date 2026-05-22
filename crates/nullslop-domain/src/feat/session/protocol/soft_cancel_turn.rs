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

//! Soft cancel turn command.
//!
//! Requests graceful termination of the current agentic turn. The turn ends
//! at the next natural pause point (tool batch completed or stream completed)
//! rather than immediately. This enables mid-turn auto-compaction: the
//! compaction actor enqueues `CompactionNeeded` and emits `SoftCancelTurn`,
//! causing the turn to stop gracefully so compaction can run.

use serde::{Deserialize, Serialize};

use crate::protocol::{CommandMsg, SessionId};

/// Request graceful termination of the current turn.
///
/// The session actor sets a flag. At the next natural pause point
/// (`ToolBatchCompleted` or `StreamCompleted`), the actor checks the flag
/// and ends the turn (→ Idle) instead of continuing. The `SessionPhaseChanged(Idle)`
/// event fires, allowing the QueueActor to pop the already-enqueued `CompactionNeeded`.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("session")]
pub struct SoftCancelTurn {
    /// The session whose turn should be cancelled.
    pub session_id: SessionId,
}
