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

//! Republishes the skills picker when a discovery scan reports new skills.
//!
//! The picker is a slice cell, so a worker holding session state cannot write
//! it directly — it holds no handle to the slice registry. Instead the scan
//! broadcasts [`SkillsLoaded`], and this listener, which is spawned by the
//! skills slice with a handle to its own cell, rebuilds the rows.
//!
//! Keeping the write on this side of the boundary is what makes the picker
//! slice-owned: the kernel publishes a discovery event and knows nothing about
//! which slice renders the result.

use jinn_skills_msg::{SkillPickerState, SkillsLoaded};
use jinn_slices::cell::TypedCell;
use trouper::actor::{MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;

/// Republishes the picker's rows whenever a skills scan reports a result.
#[derive(Debug)]
pub struct SkillPickerRepublisherActor {
    /// The picker's cell, written on every scan result.
    cell: TypedCell<SkillPickerState>,
}

impl SkillPickerRepublisherActor {
    /// The registry path this actor is published under.
    pub const PATH: &'static str = "jinn.skills.picker-republisher";

    /// Builds a republisher over the picker's own cell.
    #[must_use]
    pub fn new(cell: TypedCell<SkillPickerState>) -> Self {
        Self { cell }
    }
}

impl ServiceActor for SkillPickerRepublisherActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "ServiceActor::start is async by signature; this impl never awaits"
    )]
    async fn start(
        _args: &trouper::json::Json,
    ) -> Result<Self, error_stack::Report<RegistryError>> {
        // Never called: spawned via `start_with` (typed deps cannot ride
        // JSON args).
        Err(error_stack::Report::new(RegistryError::InvalidSpec)
            .attach("SkillPickerRepublisherActor spawns via start_with"))
    }
}

impl MsgHandler<SkillsLoaded> for SkillPickerRepublisherActor {
    /// Repaints the picker's rows when a skills scan reports a result.
    async fn handle(&mut self, msg: &SkillsLoaded, _ctx: &mut MsgCtx<'_>) {
        // A failed scan reports no skills; republishing would blank the picker.
        if msg.error.is_some() {
            return;
        }
        crate::skill_picker_routes::republish_from_discovery(&self.cell, &msg.skills);
    }
}
