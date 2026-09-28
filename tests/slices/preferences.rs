//! Preferences slice-composition tests.

use jinn_kernel::common::state::State;
use jinn_preferences::pruner_accumulation_scope;
use jinn_preferences::pruner_accumulation_slot;
use jinn_slices::OverlayViews;
use jinn_slices::SliceHost;
use jinn_slices::view::Viewport;
use ratatui_which_key::NodeResult;

#[rstest::rstest]
#[tokio::test]
async fn activation_registers_pruner_popup_and_normal_mode_opener() {
    // Given shared registries seeded by the cell catalog, as production
    // boot does, and a preferences slice host.
    let slices = jinn_slices::Slices::new();
    jinn_cell_catalog::register_all_cells(&slices);
    let mut viewport = Viewport::new();
    let overlay_views = OverlayViews::new();
    let routes = jinn_slices::KeyRoutes::new();
    let system = trouper::system::ActorSystem::new(trouper::system::SystemConfig::production());
    let services = jinn_kernel::Services::new_fake().await;
    let state = State::new(jinn_kernel::AppState::default_with_scope_focus());
    let mut host = SliceHost::new(&slices, &mut viewport, &overlay_views, &routes, &system);

    // When preferences activation registers the popup surfaces.
    jinn_preferences::activate(&mut host, &system, services, state);
    let mut keymap = jinn_tui::keymap::init();
    jinn_tui::keymap_gen::bind_route_rows(&routes, &mut keymap);

    // Then the dynamic cell, hook, geometry, view, and selectability resolve.
    let scope = pruner_accumulation_scope();
    assert!(slices.slot_type(&pruner_accumulation_slot()).is_some());
    assert!(routes.input_hook(&scope).is_some());
    assert!(slices.overlay(&scope).is_some());
    assert!(slices.overlay_slot(&scope).is_some());
    assert!(slices.overlay_selectable(&scope));
    assert!(overlay_views.view(&scope).is_some());

    // And generated normal-mode keys resolve to the preferences opener.
    let plain = |key| jinn_kernel::KeyEvent {
        key: jinn_kernel::Key::Char(key),
        modifiers: jinn_kernel::Modifiers::none(),
    };
    let result = keymap
        .navigate(
            &[plain('g'), plain('c'), plain('p')],
            &jinn_tui::Scope::Normal,
        )
        .expect("gcp path exists in the generated normal-mode keymap");
    let NodeResult::Leaf { action } = result else {
        panic!("gcp must be a leaf, got {result:?}");
    };
    let jinn_kernel::KernelIntent::Dynamic(dynamic) = action else {
        panic!("gcp must resolve to a dynamic intent, got {action:?}");
    };
    assert_eq!(dynamic.slice, pruner_accumulation_scope());
    assert_eq!(dynamic.action, "open-pruner-accumulation");
}
