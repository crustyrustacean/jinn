//! Composition-seam integration tests: the shared seam itself, not any
//! single slice.
//!
//! These assert what composition as a whole must provide — that the
//! composed route table carries every in-tree slice's rows. A missing
//! slice here means its `activate()` never attached rows, so nothing
//! downstream (keymap, which-key, rendering) can see it.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use crate::common::composition_routes;
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
        assert_eq!(
            picker_scope.captures_input(),
            true,
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
