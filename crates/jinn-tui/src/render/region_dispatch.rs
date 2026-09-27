//! Draws one region of the chat layout by asking the slice that owns it.
//!
//! Composition decides *where* each region is and *in what order* the
//! regions stack. It does not decide what is painted in them: each
//! slice registered a draw function for its region at activation, and
//! this dispatches to it. An unregistered region paints nothing, which
//! is the same degradation an unregistered overlay has always had.

use jinn_kernel::RenderCtx;
use jinn_kernel::common::app_state::AppState;
use jinn_slices::render_slot::RenderSlots;
use jinn_slices::{DrawTarget, Region};
use ratatui::Frame;

/// Draws `region` into `target` if any slice registered for it.
///
/// A region with no registered draw function is a no-op — the same
/// silent degradation the element lookup had before, and the reason
/// the startup pairing check exists. Selection is the draw function's
/// own decision: it pushes onto `rects` when the region supports it.
pub fn draw_region(
    slots: &RenderSlots<AppState>,
    region: Region,
    frame: &mut Frame<'_>,
    target: DrawTarget,
    ctx: &RenderCtx<'_>,
    rects: &mut Vec<ratatui::layout::Rect>,
) {
    if let Some(draw) = slots.draw(region) {
        draw(frame, target, ctx, rects);
    }
}
