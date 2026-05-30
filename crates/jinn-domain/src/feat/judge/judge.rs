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

//! Judge data model.
//!
//! A parsed judge definition ready for use as a session's system prompt.
//! Judges are markdown files with TOML frontmatter discovered from
//! both user (`~/.config/jinn/judges/`) and system (`/usr/share/jinn/judges/`)
//! directories.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::protocol::SessionId;

/// A parsed judge definition ready for use in a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Judge {
    /// Unique judge name (from frontmatter).
    pub name: String,
    /// Short description for the picker UI.
    pub description: String,
    /// The judge body — the actual system prompt text.
    pub body: String,
    /// Optional model override for the judge session.
    pub model: Option<String>,
    /// Whether this judge automatically resets history before each evaluation cycle.
    /// Set from the judge file's frontmatter. Per-session overrides live in [`JudgeMeta`].
    pub auto_reset: bool,
    /// File path this judge was loaded from.
    pub file_path: PathBuf,
}

/// Metadata stored on a judge session to identify it and link it to its origin.
///
/// Presence of this struct (as `Option<JudgeMeta>`) is the flag that indicates
/// a session is a judge. All judge-specific behavior checks `judge.is_some()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeMeta {
    /// The session this judge is monitoring.
    pub origin_session: SessionId,
    /// Whether the judge is actively monitoring. When false, the judge
    /// will not be triggered on origin IDLE transitions.
    ///
    /// Set to `false` by `task_complete` tool handler.
    /// Remains `true` after `task_incomplete` (judge stays attached).
    pub is_attached: bool,
    /// The name of the judge definition file used to create this session.
    pub judge_name: String,
    /// Per-session override for auto-reset behavior.
    ///
    /// `None` means use the judge file's default (`Judge::auto_reset`).
    /// `Some(true/false)` means the user explicitly toggled it in the sidebar.
    #[serde(default)]
    pub auto_reset: Option<bool>,
}
