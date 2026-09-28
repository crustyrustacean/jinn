//! The quake-bar slice — a full-width drop-down console overlay.
//!
//! Browses the session + global command log with a vim-ish keymap and
//! submits inline commands back into the log. Owns its state cell,
//! canvas actor, crossing command, route rows, and overlay renderer.
//! Activates through the host [`SliceHost`] verbs.

pub mod canvas_actor;
pub mod command;
pub mod intent;
pub mod render;
pub mod state;

pub use canvas_actor::QuakeBarCanvasActor;
pub use command::SubmitQuakeBarCommand;
pub use intent::attach_quake_bar_rows;
pub use intent::register_quake_input_hook;
pub use state::QuakeBarState;
pub use state::quake_bar_slot;
pub use state::quake_scope;

use jinn_slices::SliceHost;
use jinn_slices::SliceScopeId;

/// The dynamic focus scope the quake bar's rows live under.
///
/// Kept here for composition-side keymap assertions; the canonical
/// definition lives with the state it scopes.
#[must_use]
pub fn quake_overlay_scope() -> SliceScopeId {
    quake_scope()
}

/// Activates the quake bar: resolve the cell, spawn the canvas actor on
/// the trouper fabric, stage the forward route, attach the rows, and
/// register the overlay geometry + slot + renderer.
///
/// The cell is not minted here: the shared cell catalog
/// (`jinn_cell_catalog::register_all_cells`) registers every slice cell in
/// one place, before any slice activates, so this resolves the handle the
/// same way every other consumer does.
///
/// The forward route is staged here because this slice owns the
/// crossing command's Rust type; composition drains the staged set
/// into the kernel-side relays.
///
/// # Panics
///
/// Panics if the catalog has not run — the rows and input hook would be
/// wired to an absent cell, and the overlay would render nothing.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(host: &mut SliceHost<'_, jinn_slices::RenderFacts>) {
    let cell = host
        .slices()
        .reader::<QuakeBarState>(&quake_bar_slot())
        .expect("the cell catalog registers the quake-bar slot before any slice activates");
    canvas_actor::QuakeBarCanvasActor::spawn(host.system(), &cell);
    intent::attach_quake_bar_rows(host.key_routes(), &cell);
    intent::register_quake_input_hook(host.key_routes(), &cell);
    host.register_overlay(quake_scope(), std::sync::Arc::new(quake_overlay_rect));
    host.register_overlay_slot(quake_scope(), quake_bar_slot());
    host.register_overlay_view(quake_scope(), std::sync::Arc::new(render::render_quake_bar));
}

/// The quake bar overlay's screen rect for a frame of `area`.
///
/// # Panics
///
/// Never panics: `quake_overlay_height` clamps to the terminal height.
///
/// Full width, top-docked; height sized to the content (header + data
/// rows + log + input) by the renderer, so the geometry returns the
/// full-width top band and the renderer clamps within it.
#[must_use]
pub fn quake_overlay_rect(area: &ratatui::layout::Rect) -> Option<ratatui::layout::Rect> {
    let height = render::quake_overlay_height(area.height);
    Some(ratatui::layout::Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: height.min(area.height),
    })
}
