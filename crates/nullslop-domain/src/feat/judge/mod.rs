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

pub mod judge;
pub mod judge_scan_actor;
pub mod loader;
pub mod protocol;

pub use judge::{Judge, JudgeMeta};
pub use judge_scan_actor::{JudgeScanActor, JudgeScanActorDeps};
pub use loader::{parse_judge_file, scan_judges_dir, scan_judges_merged};
pub use protocol::{JudgesLoaded, RescanJudges};
