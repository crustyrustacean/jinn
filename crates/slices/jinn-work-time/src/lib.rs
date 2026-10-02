//! The work-time slice — how long each session has been working.
//!
//! Answers the question the per-entry timings cannot: a session streams
//! tokens, waits on tools, blocks on its own subagents, and returns to idle,
//! and none of that span is reconstructible from when one entry finished and
//! the next was dispatched.
//!
//! Owns the shared [`WorkingTimeState`] cell ([`work_time_slot`]) and the
//! monitor actor that is the single writer of every working interval. The
//! interval type itself is shared vocabulary in `jinn-core-types`, because the
//! session-store projection, the tree aggregate, and the status bar all read
//! it — none of which may depend on this slice's message crate.

pub mod monitor_actor;

use jinn_slices::SliceHost;

pub use jinn_work_time_msg::WorkingTimeState;
pub use jinn_work_time_msg::work_time_slot;
pub use monitor_actor::WorkTimeMonitorActor;

/// Activates the slice: spawns the monitor on trouper, bound to the shared
/// interval cell.
///
/// The cell is not minted here: the shared cell catalog
/// (`jinn_cell_catalog::register_all_cells`) registers every slice cell in one
/// place, so this resolves the value out of it like every other consumer.
///
/// Takes no services: a monitor's whole world is the cell, and asking the
/// caller for a `Services` it would not use forces a clone on the boot path
/// and a borrow conflict at every call site that already holds `ctx`.
///
/// # Panics
///
/// Panics if the catalog has not run — the monitor would then own a private
/// cell while every reader resolves none, and the slice would look like it
/// records working time while recording nothing.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(host: &mut SliceHost<'_, jinn_slices::RenderFacts>) {
    let cell = host
        .slices()
        .reader::<WorkingTimeState>(&work_time_slot())
        .expect("the cell catalog registers the work-time slot before any slice activates");

    let _monitor_path = WorkTimeMonitorActor::spawn(host.system(), cell);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;

    #[rstest::rstest]
    fn activate_resolves_the_catalogued_cell() {
        // Given a registry seeded through the same catalog production boot uses.
        let slices = jinn_slices::Slices::new();
        jinn_cell_catalog::register_all_cells(&slices);

        // Then the work-time slot resolves to the payload type.
        let cell = slices.reader::<WorkingTimeState>(&work_time_slot());
        assert!(
            cell.is_some(),
            "the catalog must register the work-time slot"
        );
    }

    #[rstest::rstest]
    fn a_fresh_registry_without_the_catalog_has_no_cell() {
        // Given a registry the catalog never touched.
        let slices = jinn_slices::Slices::new();

        // Then the work-time slot resolves to nothing.
        let cell = slices.reader::<WorkingTimeState>(&work_time_slot());
        assert!(cell.is_none());
    }
}
