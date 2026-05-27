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

//! Schedule auto-compaction command.
//!
//! Sent by the CompactionActor when token threshold is exceeded.
//! Sets a flag on the session that is checked at the next turn boundary
//! (stream completed or tool batch completed). When the flag is set,
//! the session transitions directly to Compacting instead of continuing
//! the turn or returning to Idle.

use serde::{Deserialize, Serialize};

use crate::protocol::{CommandMsg, SessionId};

/// Request auto-compaction at the next turn boundary.
///
/// Unlike `SoftCancelTurn` (which terminates the turn → Idle), this command
/// sets a flag that causes the session to transition **directly** to
/// `Compacting` at the next pause point — never passing through `Idle`.
/// This prevents the JudgeCoordinatorActor from firing during auto-compaction.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("session")]
pub struct ScheduleAutoCompaction {
    /// The session that should auto-compact at the next turn boundary.
    pub session_id: SessionId,
}
