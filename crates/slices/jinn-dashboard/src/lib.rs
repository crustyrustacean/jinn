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

pub mod bridge;
pub mod canvas_actor;
pub mod contracts;
pub mod fabric_events;
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

use jinn_slices::SliceScopeId;
use jinn_slices::SlotKey;
use jinn_slices::SlotTaken;

/// Services the dashboard's `activate` reads. Declared as parameters —
/// never the kernel's `Services` — so this crate cannot grow a kernel
/// dependency.
pub struct SliceCtx<'a> {
    /// The slices registry: cell + tab-scope registration.
    pub slices: &'a jinn_slices::Slices,
    /// The route table: row attachment.
    pub key_routes: &'a jinn_slices::KeyRoutes,
    /// The viewport: view registration.
    pub viewport: &'a mut jinn_slices::view::Viewport,
    /// The trouper system: actor spawning.
    pub trouper_system: &'a trouper::system::ActorSystem,
}

/// Activates the dashboard slice: mints the cell, spawns the canvas
/// actor (subscribe is the readiness point, so no lifecycle event from
/// subsequently spawned actors is missed), attaches the route rows,
/// registers the view, and declares the tab.
///
/// One call from composition (launch/wiring) is the slice's entire
/// integration surface; commenting it out removes the slice with no
/// other edits.
///
/// # Errors
///
/// Returns [`SlotTaken`] if the dashboard cell is already registered —
/// double activation is a wiring bug.
pub fn activate(ctx: &mut SliceCtx<'_>) -> Result<TypedCellRef, ActivationError> {
    // Mint the cell: the one write handle goes into the canvas actor;
    // renderer and intent router resolve read handles only.
    let cell = ctx
        .slices
        .register(dashboard_slot(), DashboardState::new())?;

    // Spawn FIRST — the dashboard must be subscribed to its topics
    // before any other actor fires lifecycle events. `subscribe`
    // registers the topic cursors synchronously, so events published
    // after this point cannot be missed, leaving no entries stuck on
    // "Starting". The forward relays (drained in composition) feed the
    // topics from the kameo bus.
    canvas_actor::DashboardCanvasActor::spawn(ctx.trouper_system, &cell);

    // Route rows + view + tab declaration.
    attach_dashboard_rows(ctx.key_routes);
    let view_result = ctx.viewport.register(DashboardView::new(), ctx.slices);
    view_result.map_err(ActivationError::ViewSlot)?;
    ctx.slices
        .register_tab_scope(dashboard_scope(), dashboard_slot());
    Ok(TypedCellRef { cell })
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

/// The cell handle `activate` hands back to composition (currently
/// unused; the canvas actor holds the write handle).
pub struct TypedCellRef {
    /// The dashboard's minted cell.
    #[expect(dead_code, reason = "activation proof; the actor owns the live handle")]
    cell: jinn_slices::TypedCell<DashboardState>,
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
