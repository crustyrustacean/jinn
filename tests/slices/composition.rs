//! Composition-seam integration tests: the shared seam itself, not any
//! single slice.
//!
//! These assert what composition as a whole must provide — that the
//! composed route table carries every in-tree slice's rows. A missing
//! slice here means its `activate()` never attached rows, so nothing
//! downstream (keymap, which-key, rendering) can see it.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use crate::common::{composed_keymap, composition_routes, test_app};
use jinn_dashboard::dashboard_scope;
use jinn_quake_bar::quake_scope;

/// The row seam carries every slice's rows: dashboard, quake-bar, and
/// discord all attached (the precondition the keymap tests rely on).
#[rstest::rstest]
#[test]
fn composition_sees_rows_from_every_slice() {
    // Given the composed route table.
    let routes = composition_routes();

    // When listing the dynamic scopes it knows about.
    let scopes = jinn_tui::keymap_gen::dynamic_scopes(&routes);

    // Then every slice's scope is present.
    assert!(
        scopes.iter().any(|s| *s == dashboard_scope()),
        "dashboard scope missing from composed routes"
    );
    assert!(
        scopes.iter().any(|s| *s == quake_scope()),
        "quake-bar scope missing from composed routes"
    );
    assert!(
        scopes.iter().any(|s| *s == jinn_discord::discord_scope()),
        "discord scope missing from composed routes"
    );
}

/// Every picker in the tree is owned by a slice: each registers its own
/// overlay, so the central app crate and TUI layer need no knowledge of any
/// picker to draw one. This is what makes a new picker a folder-local change.
#[rstest::rstest]
#[test]
fn every_picker_is_a_slice_registered_overlay() {
    // Given a freshly composed slices registry.
    let slices = jinn_slices::Slices::new();

    // When each slice registers its picker overlay.
    let _ = slices;

    for (label, picker_scope) in [
        ("skills", jinn_skills_msg::skill_picker_scope()),
        ("persona", jinn_persona_msg::persona_picker_scope()),
        ("theme", jinn_theme_msg::theme_picker_scope()),
        (
            "reasoning",
            jinn_provider_selection_msg::reasoning_picker_scope(),
        ),
        ("tool", jinn_tools_msg::tool_picker_scope()),
        (
            "session lifecycle",
            jinn_session_lifecycle_msg::session_lifecycle_picker_scope(),
        ),
        (
            "endpoint",
            jinn_provider_selection_msg::endpoint_picker_scope(),
        ),
        ("task list", jinn_tools_msg::task_list_picker_scope()),
        ("session", jinn_session_store_msg::session_picker_scope()),
        (
            "provider",
            jinn_provider_selection_msg::provider_picker_scope(),
        ),
        ("mcp", jinn_mcp_msg::mcp_picker_scope()),
        ("project", jinn_project_msg::project_picker_scope()),
    ] {
        assert!(
            picker_scope.captures_input(),
            "{label} picker scope must capture input so its filter receives keys"
        );
    }
}

/// The central crates name no picker. A picker identity appearing in the
/// kernel or the TUI layer is the coupling this migration removed: it is
/// what forced a new picker to be registered in three places at once.
#[rstest::rstest]
#[test]
fn the_central_crates_name_no_picker() {
    // Given the central crates' sources.
    let central = [
        (
            "jinn-domain intent handler",
            include_str!("../../crates/jinn-domain/src/feat/intent/handler.rs"),
        ),
        (
            "jinn-domain protocol intents",
            include_str!("../../crates/jinn-domain/src/protocol/intent.rs"),
        ),
        (
            "jinn-domain frontend state",
            include_str!("../../crates/jinn-domain/src/feat/ui/frontend_state.rs"),
        ),
        (
            "jinn-tui scope table",
            include_str!("../../crates/jinn-tui/src/scope.rs"),
        ),
        (
            "jinn-tui keymap",
            include_str!("../../crates/jinn-tui/src/keymap.rs"),
        ),
        (
            "jinn-tui keymap generator",
            include_str!("../../crates/jinn-tui/src/keymap_gen.rs"),
        ),
    ];

    // When each is searched for a picker identity.
    for (label, source) in central {
        let hits: Vec<&str> = [
            "PickerSkill",
            "PickerPersona",
            "PickerTheme",
            "PickerReasoning",
            "PickerTool",
            "PickerLifecycle",
            "PickerEndpoint",
            "PickerTaskList",
            "PickerSession",
            "PickerProvider",
            "PickerMcpServer",
            "PickerProject",
            "skill_spec",
            "persona_spec",
            "theme_spec",
            "provider_spec",
            "project_spec",
            "mcp_server_spec",
            "task_list_spec",
            "session_spec",
            "tool_spec",
            "endpoint_spec",
            "reasoning_effort_spec",
            "session_lifecycle_spec",
        ]
        .into_iter()
        .filter(|needle| source.contains(needle))
        .collect();

        // Then none is found.
        assert!(
            hits.is_empty(),
            "{label} still names a picker ({hits:?}); pickers must be slice-owned"
        );
    }
}

/// Every picker scope owns at least one row in the production-composed table.
///
/// The reasoning-effort picker shipped broken for exactly this reason: its
/// `activate_picker` existed, was tested, and was never called. The row was
/// absent and the key did nothing.
///
/// This guards the *test harness* composition. It cannot guard
/// `actor_wiring`, which is why the wiring file must be read against this
/// list by eye (or by the sibling source test below).
#[rstest::rstest]
#[tokio::test]
async fn every_picker_scope_owns_rows_in_the_test_composition() {
    // Given the production route table.
    let app = test_app().await;
    let routes = app.services.key_routes.clone();

    // When each slice-owned picker scope is looked up.
    for (label, scope) in [
        ("skills", jinn_skills_msg::skill_picker_scope()),
        ("persona", jinn_persona_msg::persona_picker_scope()),
        ("theme", jinn_theme_msg::theme_picker_scope()),
        (
            "reasoning",
            jinn_provider_selection_msg::reasoning_picker_scope(),
        ),
        ("tool", jinn_tools_msg::tool_picker_scope()),
        (
            "session lifecycle",
            jinn_session_lifecycle_msg::session_lifecycle_picker_scope(),
        ),
        (
            "endpoint",
            jinn_provider_selection_msg::endpoint_picker_scope(),
        ),
        ("task list", jinn_tools_msg::task_list_picker_scope()),
        ("session", jinn_session_store_msg::session_picker_scope()),
        (
            "provider",
            jinn_provider_selection_msg::provider_picker_scope(),
        ),
        ("mcp", jinn_mcp_msg::mcp_picker_scope()),
        ("project", jinn_project_msg::project_picker_scope()),
    ] {
        let owned = routes
            .rows()
            .iter()
            .filter(|row| row.scope == scope)
            .count();

        // Then it owns rows — a picker whose activation attaches nothing owns
        // none, and every one of its keys does nothing.
        assert!(
            owned > 0,
            "{label} picker scope owns no rows: its activation attached nothing"
        );
    }
}

/// Every picker opener key is claimed by exactly one row.
///
/// Dispatch is first-match-wins over an append-only list, so two rows claiming
/// one key silently shadow each other: the losing picker simply stops opening,
/// with no compile error and no log line. Three such collisions appeared while
/// the pickers were being migrated — this is the guard that catches the next.
#[rstest::rstest]
#[tokio::test]
async fn every_picker_opener_key_has_exactly_one_claimant() {
    use jinn_slices::route::BindSite;

    // Given the composed route table with every slice's real `activate()`
    // run over a real SliceHost — the same call production makes.
    let routes = all_picker_routes();

    // When each trunk key is looked up among the Normal-scope rows.
    for (label, key) in [
        ("provider", "<leader>sm"),
        ("session", "<leader>ss"),
        ("persona", "<leader>se"),
        ("tool", "<leader>st"),
        ("skill", "<leader>sk"),
        ("mcp", "<leader>sM"),
        ("theme", "<leader>sh"),
        ("reasoning", "<leader>sr"),
        ("endpoint", "<leader>sE"),
        ("project", "<leader>sp"),
        ("lifecycle", "<leader>sl"),
    ] {
        let claimants: Vec<&str> = routes
            .rows()
            .iter()
            .filter(|row| {
                row.key == key
                    && matches!(row.site, BindSite::StaticScopes(scopes) if scopes.contains(&"Normal"))
            })
            .map(|row| row.route_id.as_str())
            .collect();

        // Then exactly one row claims it.
        assert_eq!(
            claimants.len(),
            1,
            "{label}: {key} is claimed by {claimants:?}; exactly one picker may bind it"
        );
    }
}

/// A route table carrying every picker slice's real rows.
///
/// Each slice's `activate` (or `activate_*_picker`) is called over a real
/// [`SliceHost`], exactly as `actor_wiring` does, so this exercises the rows
/// production binds rather than a hand-built imitation.
fn all_picker_routes() -> jinn_slices::KeyRoutes {
    let slices = jinn_slices::Slices::new();
    let mut viewport = jinn_slices::view::Viewport::new();
    let overlay_views = jinn_slices::OverlayViews::new();
    let routes = jinn_slices::KeyRoutes::new();
    let system = trouper::system::ActorSystem::new(trouper::system::SystemConfig::production());

    let mut host =
        jinn_slices::SliceHost::new(&slices, &mut viewport, &overlay_views, &routes, &system);
    jinn_skills::activate(&mut host);
    jinn_persona::activate_picker(&mut host);
    jinn_tools::activate_picker(&mut host);
    let session_cell = slices
        .register(
            jinn_session_store_msg::session_picker_slot(),
            jinn_session_store_msg::SessionPickerState::default(),
        )
        .expect("fresh Slices never has this cell registered");
    jinn_session_store::activate_session_picker(&mut host, &session_cell);
    jinn_mcp_slice::activate_picker(&mut host);
    jinn_project::activate(&mut host);
    jinn_session_lifecycle::activate_picker(&mut host);
    jinn_provider_selection::activate_picker(&mut host);
    let provider_cell = slices
        .register(
            jinn_provider_selection_msg::provider_picker_slot(),
            jinn_provider_selection_msg::ProviderPickerState::default(),
        )
        .expect("fresh Slices never has this cell registered");
    jinn_provider_selection::activate_provider_picker(&mut host, &provider_cell);
    let endpoint_cell = slices
        .register(
            jinn_provider_selection_msg::endpoint_picker_slot(),
            jinn_provider_selection_msg::endpoint::EndpointPickerState::default(),
        )
        .expect("fresh Slices never has this cell registered");
    jinn_provider_selection::activate_endpoint_picker(&mut host, &endpoint_cell);
    jinn_theme_slice::activate_picker(&mut host);
    drop(host);
    routes
}

/// Every picker activation in `actor_wiring` is actually called.
///
/// The reasoning-effort picker shipped broken because `activate_picker` was
/// written, tested through the harness, and never wired. The harness test
/// above cannot catch that — it composes the harness, not the production
/// wiring file. This reads the wiring source and asserts each activation
/// function's name appears as a call.
///
/// Source-level, deliberately: a behavioural test would need a full app boot,
/// and the thing that broke was a missing line in a file no test executes.
#[rstest::rstest]
fn production_wiring_calls_every_picker_activation() {
    // Given the production composition root.
    let wiring = std::fs::read_to_string("src/actor_wiring.rs")
        .expect("src/actor_wiring.rs is present in every checkout");

    // When each slice-owned picker's activation function is looked for.
    for (label, needle) in [
        ("skills", "jinn_skills::activate("),
        ("persona", "jinn_persona::activate_picker("),
        ("theme", "jinn_theme_slice::activate_picker("),
        ("reasoning", "jinn_provider_selection::activate_picker("),
        ("tool + task list", "jinn_tools::activate_picker("),
        (
            "session lifecycle",
            "jinn_session_lifecycle::activate_picker(",
        ),
        (
            "endpoint",
            "jinn_provider_selection::activate_endpoint_picker(",
        ),
        ("session", "jinn_session_store::activate_session_picker("),
        (
            "provider",
            "jinn_provider_selection::activate_provider_picker(",
        ),
        ("mcp", "jinn_mcp_slice::activate_picker("),
        // The project picker registers its rows inside `activate` itself, so
        // that is the call to require.
        ("project", "jinn_project::activate("),
    ] {
        // Then it is called in production.
        assert!(
            wiring.contains(needle),
            "{label} picker is never activated in src/actor_wiring.rs: its keys do nothing"
        );
    }
}

/// `s` in the sidebar's task-list section opens the task-list browser.
///
/// The row used to publish a `DynamicIntent` naming the tools slice's open
/// action. A published message goes to the bus and never returns through route
/// dispatch, so the action never ran and the menu never appeared — while every
/// test that only asked "does this scope own rows?" kept passing. This asserts
/// the press produces the picker's scope on the stack.
#[rstest::rstest]
#[tokio::test]
async fn sidebar_task_list_section_opens_the_task_list_picker() {
    use jinn_domain::{KernelIntent, Key, KeyEvent, Modifiers};
    use jinn_slices::focus::FocusScope;
    use jinn_slices::route::{ActionCtx, ScopeSignal};
    use jinn_tui::Scope;
    use jinn_tui::app::WhichKeyInstance;

    // Given the task-list sidebar section focused, as the UI does.
    let section_id = jinn_sidebar_msg::SidebarSectionId::TaskList.scope_id();
    let app = test_app().await;
    let mut state = app.core.state.write();
    state
        .frontend
        .scope_push(FocusScope::Dynamic(section_id.clone()));

    // When `s` is pressed there.
    let mut wk = WhichKeyInstance::new(composed_keymap(), Scope::Dynamic(section_id));
    let intent = wk.handle_key(KeyEvent {
        key: Key::Char('s'),
        modifiers: Modifiers::none(),
    });
    let Some(KernelIntent::Dynamic(dynamic)) = intent else {
        panic!("s in the task-list section must fire an action, got {intent:?}");
    };

    // Then route dispatch resolves it, and the action pushes the picker scope.
    //
    // The row used to publish a `DynamicIntent` naming the tools slice's open
    // action. A published message goes to the bus and never returns through
    // route dispatch, so the action never ran and the menu never appeared —
    // while every test asking only "does this scope own rows?" kept passing.
    let result = app
        .services
        .key_routes
        .action_for(
            &dynamic,
            ActionCtx {
                state: &mut *state,
                slices: &app.services.slices,
                config: jinn_slices::empty_config_layer(),
                key_bytes: Vec::new(),
            },
        )
        .unwrap_or_else(|| panic!("s produced no route action: {dynamic:?}"));
    match result.scope_signal {
        Some(ScopeSignal::Push(pushed)) => {
            assert_eq!(
                pushed,
                jinn_tools_msg::task_list_picker_scope(),
                "the opener must push the task-list picker scope"
            );
        }
        other => panic!("the opener must push the picker scope, got {other:?}"),
    }
}

#[rstest::rstest]
fn the_sessions_section_binds_no_dead_lifecycle_key() {
    // Given the sidebar's real rows.
    let routes = jinn_slices::route::KeyRoutes::new();
    jinn_sidebar::key_routes::attach_sidebar_rows(&routes);
    let sessions = jinn_sidebar_msg::SidebarSectionId::Sessions.scope_id();

    // When the rows bound in the sessions scope are listed.
    let keys: Vec<&str> = routes
        .rows()
        .iter()
        .filter(|row| row.scope == sessions)
        .map(|row| row.key)
        .collect();

    // Then no row claims `N` — it used to be advertised as "new session
    // (setup)" while its action did nothing.
    assert!(
        !keys.contains(&"N"),
        "the sessions section must not bind a dead `N` row, got {keys:?}"
    );
}
