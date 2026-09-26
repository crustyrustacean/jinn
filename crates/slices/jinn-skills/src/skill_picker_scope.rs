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

//! The skill picker's scope handle, re-exported from the vocabulary crate.
//!
//! The skills picker's scope is a dynamic slice scope, so the kernel has no
//! `Scope::PickerSkill` variant to keep in sync: the picker is a scope the
//! slice mints for itself.
//!
//! The accessor itself lives in [`jinn_skills_msg`] so that any slice can ask
//! "is the skills picker open?" without depending on this implementation crate.

pub use jinn_skills_msg::skill_picker_scope;
