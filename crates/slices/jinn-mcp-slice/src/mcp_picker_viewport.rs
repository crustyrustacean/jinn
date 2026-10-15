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

//! How many server rows fit in the inspector popup.
//!
//! The navigation keys page by half of this, so it has to be a real
//! measurement: the kernel used to compute it per frame and write it into
//! central state, which a slice-owned picker has no access to.

use ratatui::layout::Rect;

/// The rows reserved for the popup's border, filter, and status lines.
const CHROME_ROWS: u16 = 5;

/// Measures the result rows available inside the popup rectangle.
///
/// `area` is the popup rectangle handed down by the render pass (see
/// `jinn_selection_widget::compute_popup_rect`), not a terminal frame.
#[must_use]
pub fn results_viewport(area: &Rect) -> usize {
    let inner = area.height;
    usize::from(inner.saturating_sub(CHROME_ROWS)).max(1)
}
