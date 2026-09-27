//! Sidebar — the left panel with its five sections (pins, persona, task
//! list, sessions, MCP servers) plus the rename-session popup.
//!
//! The slice owns the per-section view state in one shared cell
//! (`sidebar:state`), the navigation and section logic as feature
//! modules, the resize mode, the rename popup, and the
//! [`SidebarStateActor`] — a trouper [`ServiceActor`] that clamps the
//! sessions cursor when a session closes, fed by the `jinn.sidebar`
//! forward route. Keybindings resolve through route rows attached at
//! activation; a de-activated sidebar is inert by construction (no
//! rows, no bindings, no cell, no actor).

pub mod key_routes;
pub mod overlay;
pub mod render_wiring;
pub mod sections;

pub use jinn_sidebar_msg::sidebar_sections_slot;

use jinn_kernel::common::app_state::AppState;
use jinn_slices::SliceHost;

/// Activates the slice: mints the sidebar sections cell, attaches
/// the sidebar's keybind rows plus the rename input hook, and spawns
/// the sessions-cursor clamp actor on trouper (its `.subscribe`
/// declaration of `SessionClosed` is the readiness point).
///
/// # Panics
///
/// Panics if the slot is already registered — double activation is a
/// wiring bug.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(
    host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    state: jinn_kernel::common::state::State,
) {
    let cell = host
        .register_cell(
            sidebar_sections_slot(),
            jinn_sidebar_msg::SidebarSections::default(),
        )
        .expect("sidebar slot is registered exactly once at wiring");
    key_routes::attach_sidebar_rows(host.key_routes());
    key_routes::register_rename_input_hook(host.key_routes(), &cell);
    // The rename popup renders through the overlay registry keyed on its
    // dynamic scope; its rect registers as selectable (it hosts an input).
    host.register_overlay(
        key_routes::rename_scope(),
        std::sync::Arc::new(overlay::rename_overlay_rect),
    );
    host.register_overlay_slot(key_routes::rename_scope(), sidebar_sections_slot());
    host.register_overlay_view(
        key_routes::rename_scope(),
        std::sync::Arc::new(overlay::render_rename_overlay),
    );
    host.register_overlay_selectable(&key_routes::rename_scope());

    // The sidebar's own rendering registrations: the per-scope chrome
    // hints (which accent the border and bottom line use, and which
    // scope stands a lower overlay down) and the per-frame write hooks
    // that record this slice's geometry. Stated here, once, so the
    // composition layer never matches a sidebar scope by name.
    render_wiring::register_hints(host.slices());
    render_wiring::register_pre_render_hooks(host.slices());

    // The sidebar's column, and the surfaces that overflow it. Two
    // registrations, because they are two different draws: the column is
    // part of the layout and the render pass draws it early, while the
    // floating surfaces have to land on top of the columns they reach
    // across. Registering both here is what lets the render pass ask for
    // a region by role and never name a sidebar draw function.
    //
    // The priority a floating surface registers with is a call sequence
    // while this is the only registrant of the region. A second slice
    // registering here brings a `u16` priority per registration, and the
    // layer orders by it with ties broken by call order.
    host.slices().register_render_slot::<AppState>(
        jinn_slices::Region::Sidebar,
        render_wiring::column_draw_fn(),
    );
    host.slices().register_render_slot::<AppState>(
        jinn_slices::Region::FloatingSurfaces,
        render_wiring::floating_surfaces_draw_fn(),
    );

    // The sessions-cursor clamp actor: trouper, fed by the forward
    // route staged below. Subscribe is the readiness point — through
    // the host verb, so the slice never touches the system directly.
    let _path = sections::sidebar_state_actor::SidebarStateActor::spawn(host.system(), state);
}
