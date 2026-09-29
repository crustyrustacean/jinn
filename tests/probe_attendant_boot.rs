//! Boots the attendant slice exactly as `src/bootstrap/slices.rs` does —
//! real cell catalog, real activation — and asserts launch survives.
//!
//! Regression: the report-picker slot was missing from the cell catalog.
//! Every slice-level test passed (none boots the real `activate()`), while
//! a real launch panicked at the picker's bootstrap assertion. The unit
//! tests could not see this because they resolve cells directly instead of
//! through the catalog.

use jinn_kernel::common::state::State;
use jinn_slices::OverlayViews;
use jinn_slices::SliceHost;
use jinn_slices::view::Viewport;

#[rstest::rstest]
#[tokio::test]
async fn attendant_activation_survives_the_real_cell_catalog() {
    // Given the production cell catalog, seeded before any activation.
    let slices = jinn_slices::Slices::new();
    jinn_cell_catalog::register_all_cells(&slices);
    let mut viewport = Viewport::new();
    let overlay_views = OverlayViews::new();
    let routes = jinn_slices::KeyRoutes::new();
    let system = trouper::system::ActorSystem::new(trouper::system::SystemConfig::production());
    let services = jinn_kernel::Services::new_fake().await;
    let state = State::new(jinn_kernel::AppState::default_with_scope_focus());
    let mut host = SliceHost::new(&slices, &mut viewport, &overlay_views, &routes, &system);

    // When the attendant slice activates exactly as boot does.
    jinn_attendant::activate(&mut host, state.clone(), services);

    // Then launch is not aborted: both popup surfaces resolved their cells
    // from the catalog, registered their overlays, views, and routes.
    let mut keymap = jinn_tui::keymap::init();
    jinn_tui::keymap_gen::bind_route_rows(&routes, &mut keymap);
    assert!(
        routes
            .input_hook(&jinn_attendant_msg::attendant_properties_scope())
            .is_some()
    );
    assert!(
        routes
            .input_hook(&jinn_attendant_msg::attendant_report_picker_scope())
            .is_some()
    );
}
