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

//! Judge protocol types — commands and events for judge scanning and verdicts.

use serde::{Deserialize, Serialize};

use crate::protocol::{CommandMsg, EventMsg, SessionId};

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

/// A judge's evaluation verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Verdict {
    /// The task passed evaluation.
    Pass,
    /// The task failed evaluation, with a summary of issues.
    Fail(String),
}

/// Emitted when a judge session renders a verdict on its origin.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("judge-verdict")]
pub struct JudgeVerdict {
    /// The judge session that rendered the verdict.
    pub judge_session_id: SessionId,
    /// The origin session being evaluated.
    pub origin_session_id: SessionId,
    /// The name of the judge definition.
    pub judge_name: String,
    /// The verdict (pass or fail with summary).
    pub verdict: Verdict,
}

/// Cancel a pending judge evaluation cycle.
///
/// Emitted when the user presses ESC on an origin session that is
/// Idle but busy (waiting for judge evaluations). The coordinator
/// clears its pending state, clears busy on the origin, cancels
/// any still-running judge sessions, and pushes a system message.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("judge")]
pub struct CancelPendingJudgeEvaluation {
    /// The origin session whose pending evaluation should be cancelled.
    pub origin_session_id: SessionId,
}
