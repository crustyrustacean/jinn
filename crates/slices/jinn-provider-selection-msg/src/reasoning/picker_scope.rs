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

//! The reasoning picker's identity, in the vocabulary crate.
//!
//! Lives here rather than in the `jinn-provider-selection` implementation
//! crate so any slice can ask "is the reasoning picker open?" without
//! depending on that crate for a one-line accessor. The scope is the picker's
//! public handle: a slice that wants to react to the menu being open matches
//! this id against the current focus scope, and needs no other knowledge of
//! the picker.
//!
//! The id is namespaced per picker (`provider-selection` / `reasoning`)
//! rather than per slice: the provider-selection slice will later host the
//! endpoint and provider pickers too, and each of those needs its own scope.

use jinn_slices::SliceScopeId;

/// The reasoning picker's dynamic scope (input-capturing: the filter is text).
#[must_use]
pub fn reasoning_picker_scope() -> SliceScopeId {
    SliceScopeId::new("provider-selection", "reasoning")
}
