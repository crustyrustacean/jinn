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

//! Judges — attachable evaluation sessions that review origin session output.
//!
//! Judges are markdown files with TOML frontmatter discovered from
//! both user (`~/.config/nullslop/judges/`) and system (`/usr/share/nullslop/judges/`)
//! directories. User judges override system judges of the same name.
//! Each judge defines a system prompt that instructs the LLM how to evaluate
//! the origin session's work, and which tools to use for reporting verdicts.

pub mod tools;
pub mod judge;
pub mod judge_coordinator_actor;
pub mod judge_scan_actor;
pub mod loader;
pub mod picker_entry;
pub mod protocol;

pub use judge::{Judge, JudgeMeta};
pub use judge_coordinator_actor::{JudgeCoordinatorActor, JudgeCoordinatorActorDeps, resolve_effective_auto_reset};
pub use judge_scan_actor::{JudgeScanActor, JudgeScanActorDeps};
pub use loader::{parse_judge_file, scan_judges_dir, scan_judges_merged};
pub use picker_entry::JudgePickerEntry;
pub use protocol::{JudgeVerdict, JudgesLoaded, RescanJudges, Verdict};

use nullslop_provider::ToolDefinition;

/// Returns the three judge-specific tool definitions.
///
/// These are injected into the prompt only for judge sessions
/// (when `session.judge().is_some()`).
#[must_use]
pub fn judge_tool_definitions() -> Vec<ToolDefinition> {
    tools::tool_definitions()
}
