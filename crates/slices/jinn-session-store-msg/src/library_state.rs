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

//! The sidebar Library section's cell payload and slot key.
//!
//! The session-store slice owns the cell: it refreshes it from store
//! summaries on load, membership change, archive, and session persist (a
//! member's title and `updated_at` are what a row shows). The sidebar's
//! Library section renders it.

use jinn_slices::SlotKey;
use serde::{Deserialize, Serialize};

use crate::SessionSummary;

/// The Library section's contents: every session flagged as a library member.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryState {
    /// The library's members, most recently updated first.
    pub entries: Vec<SessionSummary>,
}

/// The cell key the Library section reads.
#[must_use]
pub fn library_slot() -> SlotKey {
    SlotKey::builtin("session-store", "library")
}
