//! The dashboard slice — a feature-agnostic status surface.
//!
//! One tab-backed view listing every actor in the system: its
//! lifecycle phase, description, and the owning feature's free-form
//! status message. The dashboard is a pure sink — it subscribes to
//! generic lifecycle events and `ServiceStatusUpdate` projections and
//! never needs to know a feature exists.
//!
//! Kernel integration is exactly [`activate`], called once from
//! composition; commenting that call removes the slice with no other
//! edits (removability).

pub mod canvas_actor;
pub mod contracts;
pub mod key_routes;
pub mod nav;
pub mod state;
pub mod view;

pub use canvas_actor::DashboardCanvasActor;
pub use contracts::ServiceStatusUpdate;
pub use jinn_core_types::ActorLifecycle;
pub use key_routes::attach_dashboard_rows;
pub use key_routes::dashboard_scope;
pub use nav::DashboardNav;
pub use state::{DashboardEntry, DashboardState};
pub use view::DashboardView;

use jinn_slices::RenderFacts;
use jinn_slices::SliceHost;
use jinn_slices::SliceScopeId;
use jinn_slices::SlotKey;
use jinn_slices::SlotTaken;

/// Activates the dashboard slice: mints the cell, spawns the canvas
/// actor (subscribe is the readiness point, so no lifecycle event from
/// subsequently spawned actors is missed), attaches the route rows,
/// registers the view, and declares the tab.
///
/// One call from composition (launch/wiring) is the slice's entire
/// integration surface; commenting it out removes the slice with no
/// other edits.
///
/// The slice takes the shared [`SliceHost`] like every other slice. It
/// stays free of a `jinn-kernel` dependency because the host itself
/// lives in `jinn-slices` — the dashboard needs nothing beyond the
/// registries it already had.
///
/// # Errors
///
/// Returns [`SlotTaken`] if the dashboard cell is already registered —
/// double activation is a wiring bug.
pub fn activate(host: &mut SliceHost<'_, RenderFacts>) -> Result<(), ActivationError> {
    // Mint the cell: the one write handle goes into the canvas actor;
    // renderer and intent router resolve read handles only.
    let cell = host
        .register_cell(dashboard_slot(), DashboardState::new())
        .map_err(ActivationError::SlotTaken)?;

    // Spawn FIRST — the dashboard must be subscribed to the census schema
    // before any other actor spawns, or the first rows would be missed
    // entirely. `.handles` registers the subscription synchronously, so
    // every announcement published after this point reaches the actor.
    canvas_actor::DashboardCanvasActor::spawn(host.system(), &cell);

    // Route rows + view + tab declaration. `slices` is read before
    // `viewport` is borrowed mutably — `SliceHost::viewport` hands out a
    // mutable borrow, and `Viewport::register` needs the registry too.
    attach_dashboard_rows(host.key_routes());
    let view_result = {
        let slices = host.slices();
        host.viewport().register(DashboardView::new(), slices)
    };
    view_result.map_err(ActivationError::ViewSlot)?;
    host.register_tab_scope(dashboard_scope(), dashboard_slot());
    Ok(())
}

/// Why a dashboard activation aborted.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub enum ActivationError {
    /// The dashboard cell is already registered — double activation.
    #[error(debug)]
    SlotTaken(#[from] SlotTaken),
    /// The view could not resolve its cell — a wiring bug that must
    /// abort launch, not render blank.
    #[error(debug)]
    ViewSlot(jinn_slices::view::ViewSlotError),
}

/// The dashboard slice's slot.
///
/// Canonical key shared by composition (which activates), the renderer
/// (which resolves a read handle), and tests.
#[must_use]
pub fn dashboard_slot() -> SlotKey {
    SlotKey::builtin("dashboard", "status")
}

/// The dashboard tab's dynamic scope id.
#[must_use]
pub fn dashboard_scope_id() -> SliceScopeId {
    dashboard_scope()
}
