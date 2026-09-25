//! Session-lifecycle slice-composition tests.

use jinn_domain::common::state::State;
use jinn_session_lifecycle_msg::arg_input_scope;
use jinn_session_lifecycle_msg::arg_input_slot;
use jinn_slices::SliceHost;
use jinn_slices::route::EditIntent;
use jinn_slices::view::Viewport;

#[rstest::rstest]
#[tokio::test]
async fn activation_registers_argument_popup_surfaces() {
    // Given fresh shared slice registries and lifecycle state.
    let slices = jinn_slices::Slices::new();
    let mut viewport = Viewport::new();
    let overlay_views = jinn_domain::common::overlay_views::OverlayViews::new();
    let routes = jinn_slices::KeyRoutes::new();
    let system = trouper::system::ActorSystem::new(trouper::system::SystemConfig::production());
    let services = jinn_domain::Services::new_fake().await;
    let state = State::new(jinn_domain::AppState::default());
    let mut host = SliceHost::new(&slices, &mut viewport, &overlay_views, &routes, &system);

    // When the lifecycle slice activates.
    jinn_session_lifecycle::activate(
        &mut host,
        &services,
        state,
        jinn_session_lifecycle_msg::BuiltinRegistry::new(),
        "/bin/sh".to_owned(),
    );

    // Then the cell, input hook, overlay geometry, view, and route actions resolve.
    let scope = arg_input_scope();
    assert!(
        slices
            .reader::<jinn_session_lifecycle_msg::ArgInputState>(&arg_input_slot())
            .is_some()
    );
    assert!(routes.input_hook(&scope).is_some());
    assert!(slices.overlay(&scope).is_some());
    assert!(slices.overlay_slot(&scope).is_some());
    assert!(slices.overlay_selectable(&scope));
    assert!(overlay_views.view(&scope).is_some());
    assert!(routes.rows().iter().any(|row| {
        row.scope == scope
            && matches!(&row.outcome, jinn_slices::RouteOutcome::Action { action, .. } if *action == "confirm-lifecycle-args")
    }));
}

#[rstest::rstest]
fn registered_argument_hook_dispatches_through_dynamic_intent() {
    // Given a popup cell and a shared route table carrying lifecycle actions.
    let slices = jinn_slices::Slices::new();
    let cell = slices
        .register(
            arg_input_slot(),
            jinn_session_lifecycle_msg::ArgInputState::empty(),
        )
        .expect("fresh registry has the lifecycle argument slot free");
    let routes = jinn_slices::KeyRoutes::new();
    jinn_session_lifecycle::arg_input::attach_rows(&routes, &cell);
    jinn_session_lifecycle::arg_input::register_input_hook(&routes, &cell);

    // When a Home intent reaches the registered hook.
    cell.update(|state| state.text.set("héllo".to_owned()));
    let hook = routes
        .input_hook(&arg_input_scope())
        .expect("activation registered the lifecycle argument hook");
    let result = hook(&EditIntent::CursorHome);

    // Then the input surface consumes the key and applies the cursor edit.
    assert!(result.is_some());
    assert_eq!(cell.read().text.cursor_pos, 0);
}
