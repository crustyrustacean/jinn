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

//! Judge protocol types — commands and events for judge scanning.

use serde::{Deserialize, Serialize};

use crate::protocol::{CommandMsg, EventMsg};

use super::Judge;

/// Rescan the judges directory and reload judge definitions.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("judge")]
pub struct RescanJudges;

/// Emitted when judges have been scanned and loaded from disk.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("judge")]
pub struct JudgesLoaded {
    /// The loaded judge files.
    pub judges: Vec<Judge>,
    /// Error message if scanning failed, `None` on success.
    pub error: Option<String>,
}
