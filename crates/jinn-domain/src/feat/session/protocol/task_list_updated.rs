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

//! Task list was updated by a tool.
//!
//! Emitted by todo list mutation tools after a successful change.
//! The session actor subscribes to this event to persist the updated task list.

use serde::{Deserialize, Serialize};

use crate::protocol::{SessionId};

/// A task list mutation was applied successfully.
///
/// Broadcast after any todo list tool modifies the task list (add phase, add task,
/// complete task, postpone task, postpone to phase, or set list).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskListUpdated {
    /// The session whose task list was updated.
    pub session_id: SessionId,
}
