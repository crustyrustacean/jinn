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

//! The session-lifecycle picker's identity, in the vocabulary crate.
//!
//! Lives here rather than in the `jinn-session-lifecycle` implementation
//! crate so any slice can ask "is the session-lifecycle picker open?" without
//! depending on that crate for a one-line accessor. The scope is the picker's
//! public handle: a slice that wants to react to the menu being open matches
//! this id against the current focus scope, and needs no other knowledge of
//! the picker.
//!
//! The project picker chains into this one — `<c-enter>` on a project opens the
//! session-lifecycle menu — so this accessor is the seam that lets the
//! project slice reach it without either picker naming the other.

use jinn_slices::SliceScopeId;

/// The session-lifecycle picker's dynamic scope (input-capturing: the filter
/// is text).
#[must_use]
pub fn session_lifecycle_picker_scope() -> SliceScopeId {
    SliceScopeId::new("session-lifecycle", "picker")
}
