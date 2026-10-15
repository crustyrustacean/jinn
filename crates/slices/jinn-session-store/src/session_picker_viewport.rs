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

//! The session picker's pre-render viewport fallback.
//!
//! The render pass measures the popup's result rows itself through the
//! widget's `PickerLayout` report; this module only carries the value to
//! assume before the first frame has been drawn.

/// The row count to assume before the first render pass.
///
/// A *fallback*, not a default: the render pass overwrites it on the first
/// frame. It is deliberately the same value the kernel's own
/// `measure_active_picker_results_height` used, so paging behaves identically
/// in the frame or two before a measurement lands.
pub const FALLBACK_RESULTS_VIEWPORT: usize = 20;
