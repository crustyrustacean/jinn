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

//! Retry a session whose turn has stalled.
//!
//! Emitted by the [`StallWatchdogActor`](crate::feat::stall_watchdog_actor::StallWatchdogActor)
//! when a session in `Sending`/`Streaming` has had no chat-history activity
//! for longer than `history_stall_timeout_secs`. The handler discards any
//! partial streaming entries, pushes a system marker, and re-dispatches the
//! turn — mirroring the server-error retry path. A hung session is treated
//! identically to a hard provider error.

use serde::{Deserialize, Serialize};

use crate::protocol::SessionId;

/// Command to retry a session whose turn has stalled.
///
/// Published by the stall watchdog only while the session's stall-retry
/// budget is not yet exhausted. Once the budget is exceeded the watchdog
/// publishes `CancelStream` instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryStalledSession {
    /// The session whose turn has stalled.
    pub session_id: SessionId,
}

impl crate::common::bus::BusMessage for RetryStalledSession {}
