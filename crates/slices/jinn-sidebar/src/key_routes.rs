//! Sidebar keybind rows on the shared route table.
//!
//! Every sidebar key resolves here instead of a kernel keymap table:
//! entry keys bind in the composition scopes that can open the sidebar,
//! in-section keys bind in the section's own dynamic scope, and the
//! resize mode binds in `sidebar:resize`. A de-activated slice attaches
//! no rows, so the keys stay unbound — the sidebar is inert by
//! construction.

use jinn_kernel::common::app_state::AppState;
use jinn_kernel::protocol::IntentResult;
use jinn_slices::SliceScopeId;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, KeyRoutes, RouteId, RouteOutcome, RouteRow, ScopeSignal,
};

use crate::sections::intent as sidebar_intent;
use crate::sections::pins::pins_section as pins;
use crate::sections::rename_input::intent as rename;
use crate::sections::resize::intent as resize;
use crate::sections::section_trait::SidebarIntent;
use crate::sections::sessions;
use crate::sections::sidebar::{jump_to_section, navigate_sidebar};
use crate::sections::task_list_section as task_list;

/// The sidebar's resize-mode dynamic scope.
#[must_use]
pub fn resize_scope() -> SliceScopeId {
    jinn_sidebar_msg::SidebarSectionId::resize_scope_id()
}

/// The rename popup's dynamic scope (input-capturing).
#[must_use]
pub fn rename_scope() -> SliceScopeId {
    rename::rename_scope()
}

/// Downcasts the action context's state to the kernel's application
/// state. Sidebar actions drive concrete session behavior (activate,
/// close, archive), which needs the full state surface.
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> &'a mut AppState {
    ctx.state
        .as_any_mut()
        .and_then(|any| any.downcast_mut::<AppState>())
        .expect("sidebar route action dispatched against a non-AppState state")
}

/// Wraps a synchronous sidebar function into an [`ActionFn`].
fn sync<F>(f: F) -> ActionFn
where
    F: Fn(&mut AppState) -> IntentResult + Send + Sync + 'static,
{
    ActionFn::new(move |mut ctx| f(app(&mut ctx)))
}

/// Wraps a synchronous sidebar function that reads `jinn.toml` into an
/// [`ActionFn`].
///
/// Same as [`sync`], but the function also receives the configuration
/// layer. A route action that resolves a lifecycle's teardown or setup
/// command needs the live document; a `sync` closure would see only the
/// state and silently find nothing.
fn sync_with_config<F>(f: F) -> ActionFn
where
    F: Fn(&mut AppState, &jinn_slices::ConfigLayer) -> IntentResult + Send + Sync + 'static,
{
    ActionFn::new(move |mut ctx| {
        // Clone the handle, not the borrow: `ActionCtx` lends `&mut`
        // state, so the state lend and a config borrow cannot overlap.
        let config = ctx.config.clone();
        f(app(&mut ctx), &config)
    })
}

/// Builds one `Action` row binding `key` in `scope`. `action`/`display`
/// must be `'static` (they are the route-table key and which-key label).
fn row(
    action: &'static str,
    scope: SliceScopeId,
    key: &'static str,
    category: &'static str,
    display: &'static str,
    run: ActionFn,
) -> RouteRow {
    RouteRow {
        // Route ids are only diagnostics/composition keys here; actions
        // dispatch by (scope, action).
        route_id: RouteId::new("sidebar:row"),
        scope,
        key,
        category,
        site: BindSite::OwnScope,
        feature: "sidebar",
        outcome: RouteOutcome::Action {
            action,
            display,
            run,
        },
    }
}

/// Attaches the sidebar's keybind rows onto the shared route table.
/// Called once from the slice's `activate()`.
pub fn attach_sidebar_rows(routes: &KeyRoutes) {
    let persona = jinn_sidebar_msg::SidebarSectionId::Persona.scope_id();
    let pins_scope = jinn_sidebar_msg::SidebarSectionId::Pins.scope_id();
    let attendant_scope = jinn_sidebar_msg::SidebarSectionId::Attendant.scope_id();
    let task_list_scope = jinn_sidebar_msg::SidebarSectionId::TaskList.scope_id();
    let sessions_scope = jinn_sidebar_msg::SidebarSectionId::Sessions.scope_id();
    let mcp = jinn_sidebar_msg::SidebarSectionId::McpServers.scope_id();
    let resize = resize_scope();
    let sections = [
        persona.clone(),
        pins_scope.clone(),
        attendant_scope.clone(),
        task_list_scope.clone(),
        sessions_scope.clone(),
        mcp.clone(),
    ];

    // ---- Shared base keys (every section) ----
    for scope in &sections {
        routes.attach(row(
            "move-down",
            scope.clone(),
            "j",
            "navigation",
            "cursor down",
            sync_with_config(|state, config| {
                navigate_sidebar(&SidebarIntent::MoveDown, state, config)
            }),
        ));
        routes.attach(row(
            "move-up",
            scope.clone(),
            "k",
            "navigation",
            "cursor up",
            sync_with_config(|state, config| {
                navigate_sidebar(&SidebarIntent::MoveUp, state, config)
            }),
        ));
        routes.attach(row(
            "section-next",
            scope.clone(),
            "J",
            "navigation",
            "next section",
            sync_with_config(|state, config| {
                jump_to_section(&SidebarIntent::MoveDown, state, config)
            }),
        ));
        routes.attach(row(
            "section-prev",
            scope.clone(),
            "K",
            "navigation",
            "previous section",
            sync_with_config(|state, config| {
                jump_to_section(&SidebarIntent::MoveUp, state, config)
            }),
        ));
        routes.attach(row(
            "leave",
            scope.clone(),
            "<esc>",
            "general",
            "return to chat",
            sync(sidebar_intent::handle_sidebar_leave),
        ));
        routes.attach(row(
            "leave-chat",
            scope.clone(),
            "<c-h>",
            "navigation",
            "return to chat",
            sync(sidebar_intent::handle_sidebar_leave),
        ));
        routes.attach(RouteRow {
            route_id: RouteId::new("sidebar:quit"),
            scope: scope.clone(),
            key: "q",
            category: "general",
            site: BindSite::OwnScope,
            feature: "sidebar",
            outcome: RouteOutcome::StaticIntent(RouteId::new("sidebar:quit")),
        });
        routes.attach(RouteRow {
            route_id: RouteId::new("sidebar:ctrl-clear"),
            scope: scope.clone(),
            key: "<c-c>",
            category: "general",
            site: BindSite::OwnScope,
            feature: "sidebar",
            outcome: RouteOutcome::StaticIntent(RouteId::new("sidebar:ctrl-clear")),
        });
        routes.attach(RouteRow {
            route_id: RouteId::new("sidebar:which-key"),
            scope: scope.clone(),
            key: "?",
            category: "general",
            site: BindSite::OwnScope,
            feature: "sidebar",
            outcome: RouteOutcome::StaticIntent(RouteId::new("sidebar:which-key")),
        });
        routes.attach(row(
            "resize-enter",
            scope.clone(),
            "<c-w>",
            "navigation",
            "resize sidebar",
            sync(move |state| {
                let mut result = resize::handle_resize_enter(state);
                result.scope_signal = Some(ScopeSignal::Push(resize_scope()));
                result
            }),
        ));
    }

    // ---- Persona section ----
    routes.attach(row(
        "persona-edit",
        persona,
        "c",
        "general",
        "change persona",
        sync(pins::handle_sidebar_persona_edit),
    ));

    // ---- Pins section ----
    routes.attach(row(
        "unpin",
        pins_scope.clone(),
        "u",
        "general",
        "unpin entry",
        sync(pins::handle_pins_unpin),
    ));
    routes.attach(row(
        "pin-top",
        pins_scope.clone(),
        "t",
        "general",
        "pin to top",
        sync(|state| pins::handle_pins_pin(state, jinn_kernel::protocol::PinPosition::Top)),
    ));
    routes.attach(row(
        "pin-bottom",
        pins_scope.clone(),
        "b",
        "general",
        "pin to bottom",
        sync(|state| pins::handle_pins_pin(state, jinn_kernel::protocol::PinPosition::Bottom)),
    ));
    routes.attach(row(
        "pin-relative",
        pins_scope.clone(),
        "r",
        "general",
        "pin above/below",
        sync(|state| pins::handle_pins_pin(state, jinn_kernel::protocol::PinPosition::Relative)),
    ));
    routes.attach(row(
        "pin-cycle",
        pins_scope.clone(),
        "m",
        "general",
        "cycle pin position",
        sync(pins::handle_pins_pin_cycle),
    ));
    routes.attach(row(
        "leave-enter",
        pins_scope.clone(),
        "<enter>",
        "general",
        "return to chat",
        sync(sidebar_intent::handle_sidebar_leave),
    ));

    // ---- Sessions section ----
    routes.attach(RouteRow {
        route_id: RouteId::new("sidebar:load-subagent"),
        scope: sessions_scope.clone(),
        key: "<enter>",
        category: "general",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "sidebar",
        outcome: RouteOutcome::Action {
            action: "load-subagent",
            display: "open subagent session",
            run: sync(sessions::handle_load_subagent_session),
        },
    });
    routes.attach(row(
        "session-close",
        sessions_scope.clone(),
        "x",
        "general",
        "close session",
        sync(sessions::handle_session_close_arm),
    ));
    routes.attach(row(
        jinn_sidebar_msg::TREE_TEARDOWN_ACTION,
        sessions_scope.clone(),
        "X",
        "general",
        "teardown+archive tree",
        sync(|state| {
            sessions::handle_session_tree_action_arm(
                state,
                sessions::TreePromptAction::TeardownAndArchive,
            )
        }),
    ));
    routes.attach(row(
        "session-teardown",
        sessions_scope.clone(),
        "t",
        "general",
        "run teardown",
        sync_with_config(sessions::handle_session_teardown),
    ));
    routes.attach(row(
        "session-confirm",
        sessions_scope.clone(),
        "<enter>",
        "general",
        "activate session",
        sync(sessions::handle_session_activate),
    ));
    routes.attach(row(
        "session-rename",
        sessions_scope.clone(),
        "r",
        "general",
        "rename session",
        // The enter handler pushes the popup's dynamic scope itself.
        sync(rename::handle_rename_session_enter),
    ));

    routes.attach(row(
        "rename-confirm",
        rename_scope(),
        "<enter>",
        "input",
        "rename the session",
        sync(rename::handle_rename_session_confirm),
    ));
    routes.attach(row(
        "rename-leave",
        rename_scope(),
        "<esc>",
        "general",
        "cancel rename",
        sync(rename::handle_rename_session_leave),
    ));
    routes.attach(row(
        "rename-clear-or-leave",
        rename_scope(),
        "<c-c>",
        "general",
        "clear the title, or leave when already empty",
        sync(rename::handle_rename_session_clear_or_leave),
    ));
    routes.attach(row(
        "session-archive",
        sessions_scope.clone(),
        "a",
        "general",
        "archive session",
        sync(sessions::handle_session_archive),
    ));
    routes.attach(row(
        jinn_sidebar_msg::TREE_ARCHIVE_ACTION,
        sessions_scope.clone(),
        "A",
        "general",
        "archive subtree",
        sync(|state| {
            sessions::handle_session_tree_action_arm(state, sessions::TreePromptAction::Archive)
        }),
    ));
    routes.attach(row(
        "session-continue",
        sessions_scope.clone(),
        "c",
        "general",
        "continue session",
        sync(sessions::handle_session_continue),
    ));
    routes.attach(row(
        "session-rerun-setup",
        sessions_scope.clone(),
        "s",
        "general",
        "rerun setup",
        sync(|state| {
            sessions::handle_session_rerun_setup(state, jinn_slices::empty_config_layer())
        }),
    ));
    routes.attach(row(
        "session-new-attendant",
        sessions_scope.clone(),
        "N",
        "general",
        "new attendant of session",
        sync_with_config(sessions::attendant_actions::handle_new_attendant),
    ));
    routes.attach(row(
        "session-rerun-attendant",
        sessions_scope.clone(),
        "R",
        "general",
        "re-run attendant",
        sync(sessions::attendant_actions::handle_rerun_attendant),
    ));
    routes.attach(row(
        "session-attendant-properties",
        sessions_scope.clone(),
        "P",
        "general",
        "attendant properties",
        sync(sessions::attendant_properties::handle_open_attendant_properties),
    ));
    routes.attach(row(
        "session-terminal",
        sessions_scope.clone(),
        "T",
        "general",
        "toggle terminal",
        // Runs the term slice's handler inline. This used to publish a
        // `KernelIntent::Dynamic` message naming the term slice's action, but
        // a published message goes to the bus and no actor subscribes to
        // `KernelIntent` -- so `T` did nothing at all. `RouteResult` has no
        // local-dispatch channel, so the sidebar calls the handler the same
        // way it reaches any other cross-slice action: over `AppState`.
        ActionFn::new(|mut ctx| {
            let slices = ctx.slices;
            jinn_term::route_rows::handle_toggle_for_selected(app(&mut ctx), slices);
            IntentResult::empty()
        }),
    ));
    routes.attach(row(
        "session-insert",
        sessions_scope.clone(),
        "i",
        "general",
        "activate + insert",
        sync(sessions::handle_session_activate_insert),
    ));

    // ---- Attendants section ----
    routes.attach(row(
        "attendant-open-reports",
        attendant_scope.clone(),
        "s",
        "general",
        "browse report history",
        // Opens the report-history browser through the attendant slice's own
        // opener action — same reasoning as the task-list picker above.
        jinn_attendant::report_picker_opener(),
    ));
    routes.attach(row(
        "attendant-open-properties",
        attendant_scope.clone(),
        "P",
        "general",
        "attendant properties",
        // The same opener the sessions section binds, deliberately. The
        // properties popup is one popup, and the only thing that differs
        // between the two sections is which row the cursor is on — so one
        // handler reading the focused section keeps them from drifting. Two
        // rows with the same key are free: the route table dispatches on
        // (scope, action), and each section's keys live in its own keymap
        // layer.
        sync(sessions::attendant_properties::handle_open_attendant_properties),
    ));

    // ---- Task list section ----
    routes.attach(row(
        "task-open-picker",
        task_list_scope.clone(),
        "s",
        "general",
        "browse task list",
        // Opens the task-list browser through the tools slice's own opener
        // action. A published `DynamicIntent` would go to the bus and never
        // return through route dispatch, so the action would never run; the
        // action is called directly instead. The sidebar still holds no
        // handle to the picker's cell.
        jinn_tools::task_list_picker_opener(),
    ));
    routes.attach(row(
        "task-preview-up",
        task_list_scope.clone(),
        "<pgup>",
        "navigation",
        "preview up",
        sync(task_list::handle_preview_scroll_up),
    ));
    routes.attach(row(
        "task-preview-down",
        task_list_scope.clone(),
        "<pgdn>",
        "navigation",
        "preview down",
        sync(task_list::handle_preview_scroll_down),
    ));

    // ---- Resize mode ----
    routes.attach(row(
        "resize-expand",
        resize.clone(),
        "h",
        "general",
        "widen sidebar",
        sync(resize::handle_resize_expand),
    ));
    routes.attach(row(
        "resize-contract",
        resize.clone(),
        "l",
        "general",
        "narrow sidebar",
        sync(resize::handle_resize_contract),
    ));
    routes.attach(row(
        "resize-leave",
        resize.clone(),
        "<esc>",
        "general",
        "leave resize",
        sync(move |state| {
            let mut result = resize::handle_resize_leave(state);
            result.scope_signal = Some(ScopeSignal::PopIf(resize_scope()));
            result
        }),
    ));
    routes.attach(row(
        "resize-clear-or-leave",
        resize,
        "<c-c>",
        "general",
        "leave resize",
        sync(resize::handle_resize_leave),
    ));

    // ---- Entry keys (Normal + Input scopes) ----
    routes.attach(RouteRow {
        route_id: RouteId::new("sidebar:focus"),
        scope: sessions_scope.clone(),
        key: "<c-l>",
        category: "navigation",
        site: BindSite::StaticScopes(&["Normal", "Input"]),
        feature: "sidebar",
        outcome: RouteOutcome::Action {
            action: "focus",
            display: "focus sidebar",
            run: sync(sidebar_intent::handle_sidebar_focus),
        },
    });
    routes.attach(RouteRow {
        route_id: RouteId::new("sidebar:focus-sessions"),
        scope: sessions_scope,
        key: "<M-s>",
        category: "navigation",
        site: BindSite::StaticScopes(&["Normal", "Input"]),
        feature: "sidebar",
        outcome: RouteOutcome::Action {
            action: "focus-sessions",
            display: "focus session list",
            run: sync_with_config(sidebar_intent::handle_sidebar_focus_sessions),
        },
    });
    routes.attach(RouteRow {
        route_id: RouteId::new("sidebar:resize-mode"),
        scope: resize_scope(),
        key: "<c-w>",
        category: "navigation",
        site: BindSite::StaticScopes(&["Normal", "Input"]),
        feature: "sidebar",
        outcome: RouteOutcome::Action {
            action: "resize-mode",
            display: "resize sidebar",
            run: sync(move |state| {
                let mut result = resize::handle_resize_enter(state);
                result.scope_signal = Some(ScopeSignal::Push(resize_scope()));
                result
            }),
        },
    });
}

/// Registers the rename popup's input hook on the shared route table:
/// within the rename scope, ordinary typing and paste edit the popup's
/// in-progress text directly in the sections cell — no kernel state
/// access needed.
pub fn register_rename_input_hook(
    routes: &KeyRoutes,
    cell: &jinn_slices::cell::TypedCell<jinn_sidebar_msg::SidebarSections>,
) {
    use jinn_slices::route::{EditIntent, InputHook};

    let cell = cell.clone();
    let hook: InputHook = std::sync::Arc::new(move |intent: &EditIntent| {
        let cell = cell.clone();
        let result = match intent {
            EditIntent::InsertChar(ch) => {
                cell.update(|s| rename::insert_char(&mut s.rename_input, *ch));
                IntentResult::empty()
            }
            EditIntent::DeleteBackward => {
                cell.update(|s| rename::delete(&mut s.rename_input));
                IntentResult::empty()
            }
            EditIntent::DeleteForward => {
                cell.update(|s| rename::delete_forward(&mut s.rename_input));
                IntentResult::empty()
            }
            EditIntent::CursorLeft => {
                cell.update(|s| rename::cursor_left(&mut s.rename_input));
                IntentResult::empty()
            }
            EditIntent::CursorRight => {
                cell.update(|s| rename::cursor_right(&mut s.rename_input));
                IntentResult::empty()
            }
            EditIntent::CursorHome => {
                cell.update(|s| rename::cursor_home(&mut s.rename_input));
                IntentResult::empty()
            }
            EditIntent::CursorEnd => {
                cell.update(|s| rename::cursor_end(&mut s.rename_input));
                IntentResult::empty()
            }
            EditIntent::Paste(text) => {
                cell.update(|s| rename::paste(&mut s.rename_input, text));
                IntentResult::empty()
            }
        };
        Some(result)
    });
    routes.register_input_hook(&rename_scope(), hook);
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;
    use jinn_slices::route::EditIntent;

    fn rename_cell() -> (
        jinn_slices::Slices,
        jinn_slices::TypedCell<jinn_sidebar_msg::SidebarSections>,
    ) {
        let slices = jinn_slices::Slices::new();
        let cell = slices
            .register(
                crate::sidebar_sections_slot(),
                jinn_sidebar_msg::SidebarSections::default(),
            )
            .expect("fresh registry has the sidebar slot free");
        (slices, cell)
    }

    #[rstest::rstest]
    fn rename_input_hook_pastes_unicode_at_cursor() {
        // Given a rename cell containing Unicode text with the cursor at its start.
        let routes = KeyRoutes::new();
        let (_slices, cell) = rename_cell();
        cell.update(|sections| sections.rename_input.text.set("é界".to_owned()));
        cell.update(|sections| sections.rename_input.text.cursor_home());
        register_rename_input_hook(&routes, &cell);
        let hook = routes
            .input_hook(&rename_scope())
            .expect("rename hook is registered");

        // When bracketed paste is handled.
        let result = hook(&EditIntent::Paste("🙂".to_owned()));

        // Then the paste is consumed and inserted at the cursor.
        assert!(result.is_some());
        assert_eq!(cell.read().rename_input.text.input, "🙂é界");
    }

    #[rstest::rstest]
    fn rename_input_hook_consumes_home_and_end() {
        // Given a rename cell with the cursor at the start.
        let routes = KeyRoutes::new();
        let (_slices, cell) = rename_cell();
        cell.update(|sections| sections.rename_input.text.set("héllo".to_owned()));
        cell.update(|sections| sections.rename_input.text.cursor_home());
        register_rename_input_hook(&routes, &cell);
        let hook = routes
            .input_hook(&rename_scope())
            .expect("rename hook is registered");

        // When End and Home are handled.
        let end = hook(&EditIntent::CursorEnd);
        let end_position = cell.read().rename_input.text.cursor_pos;
        let home = hook(&EditIntent::CursorHome);

        // Then both intents are consumed and the cursor reaches each boundary.
        assert!(end.is_some());
        assert_eq!(end_position, "héllo".len());
        assert!(home.is_some());
        assert_eq!(cell.read().rename_input.text.cursor_pos, 0);
    }

    /// The rename popup's `<c-c>` route is a slice action rather than a
    /// static kernel intent, so its clear/leave behavior stays owned here.
    #[rstest::rstest]
    #[test]
    fn attach_sidebar_rows_binds_action_for_ctrl_clear_in_rename_scope() {
        // Given an empty shared route table.
        let routes = KeyRoutes::new();

        // When the sidebar's rows are attached.
        attach_sidebar_rows(&routes);

        // Then <c-c> resolves to the rename clear-or-leave action.
        assert!(routes.rows().iter().any(|row| {
            row.scope == rename_scope()
                && row.key == "<c-c>"
                && matches!(
                    &row.outcome,
                    RouteOutcome::Action { action, .. } if *action == "rename-clear-or-leave"
                )
        }));
    }

    /// `P` opens the properties popup from the sessions section, and the
    /// attendants section is the other place a user stands to edit an
    /// attendant — so the same key does the same thing there.
    #[rstest::rstest]
    #[test]
    fn attach_sidebar_rows_binds_P_for_attendant_properties_in_the_attendants_scope() {
        // Given an empty shared route table.
        let routes = KeyRoutes::new();

        // When the sidebar's rows are attached.
        attach_sidebar_rows(&routes);

        // Then `P` in the attendants scope resolves to the properties opener.
        assert!(routes.rows().iter().any(|row| {
            row.scope == jinn_sidebar_msg::SidebarSectionId::Attendant.scope_id()
                && row.key == "P"
                && matches!(
                    &row.outcome,
                    RouteOutcome::Action { action, .. } if *action == "attendant-open-properties"
                )
        }));
    }

    /// `<c-c>` in the resize scope left resize mode. It carried
    /// `RouteOutcome::StaticIntent("sidebar:quit")`, so the key quit the app
    /// instead — a row id of "resize-mode" with a quit outcome under it, which
    /// no reviewer reading the id alone would catch.
    #[rstest::rstest]
    #[test]
    fn attach_sidebar_rows_does_not_bind_ctrl_clear_to_quit_in_resize_scope() {
        // Given an empty shared route table.
        let routes = KeyRoutes::new();

        // When the sidebar's rows are attached.
        attach_sidebar_rows(&routes);

        // Then no row in the resize scope binds <c-c> to the quit intent.
        assert!(
            !routes.rows().iter().any(|row| {
                row.scope == resize_scope()
                    && row.key == "<c-c>"
                    && matches!(
                        &row.outcome,
                        RouteOutcome::StaticIntent(id) if id.as_str() == "sidebar:quit"
                    )
            }),
            "<c-c> in the resize scope must not resolve to quit"
        );
    }
}

/// The sidebar's `T` actually toggles the terminal overlay.
///
/// This row used to publish a `KernelIntent::Dynamic` message naming the term
/// slice's `toggle-for-selected` action. A published message goes to the bus,
/// and no actor subscribes to `KernelIntent` — so the message was dropped and
/// `T` did nothing. The row is now a direct call, and this test dispatches it
/// the way the composed keymap does.
#[rstest::rstest]
fn session_terminal_row_toggles_the_overlay() {
    // Given a session holding a live terminal, with the Sessions section
    // selected in the sidebar.
    use jinn_kernel::AppState;
    use jinn_session_state::ChatSessionState;
    let _ = jinn_term_msg::TERM_CONTROLS.set(jinn_term_msg::TermControls::default());
    let mut state = AppState::default_with_scope_focus();
    let slices = jinn_slices::Slices::new();
    let routes = KeyRoutes::new();
    attach_sidebar_rows(&routes);
    let session = ChatSessionState::new();
    let session_id = session.session_id().clone();
    state.session.insert(session);
    state
        .term_tabs()
        .expect("the term tabs cell is registered by wiring")
        .update(|tabs| tabs.set_live(&session_id, true));
    state
        .frontend
        .scope_swap_base(jinn_slices::FocusScope::Dynamic(
            jinn_sidebar_msg::SidebarSectionId::Sessions.scope_id(),
        ));
    state
        .frontend
        .update_sections(|s| s.sessions.selected_index = Some(0));

    // When `T` is pressed in the sidebar's sessions section.
    let intent = jinn_slices::DynamicIntent::new(
        jinn_sidebar_msg::SidebarSectionId::Sessions.scope_id(),
        "session-terminal",
        "toggle terminal",
    );
    routes
        .action_for(
            &intent,
            ActionCtx {
                state: &mut state,
                slices: &slices,
                config: jinn_slices::empty_config_layer(),
                key_bytes: Vec::new(),
            },
        )
        .expect("the sessions section binds a `T` row");

    // Then the terminal overlay is open.
    assert_eq!(
        state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(jinn_term_msg::view_scope()),
        "`T` must open the terminal overlay, not publish a message nobody reads"
    );
}

/// The popup cell a state seeded by `default_with_scope_focus` writes.
///
/// `AppState::default_with_scope_focus` already attaches the real cell
/// catalog, so the properties cell is registered — and `scope_focus` is a
/// `OnceLock`, so it cannot be swapped for a test's own registry afterwards.
/// Reading the cell off the state is therefore the only way to see what the
/// opener seeded; a locally-built cell would be a different instance that
/// the handler never writes.
fn properties_cell(
    state: &jinn_kernel::AppState,
) -> jinn_slices::TypedCell<jinn_attendant_msg::AttendantPropertiesState> {
    state
        .frontend
        .slices()
        .expect("the fixture state has a cell registry")
        .reader::<jinn_attendant_msg::AttendantPropertiesState>(
            &jinn_attendant_msg::attendant_properties_slot(),
        )
        .expect("the catalog registers the properties cell")
        .clone()
}

/// Puts `cursor` on the given sidebar section.
fn focus_section(state: &jinn_kernel::AppState, section: jinn_sidebar_msg::SidebarSectionId) {
    state
        .frontend
        .scope_swap_base(jinn_slices::FocusScope::Dynamic(section.scope_id()));
}

/// An active session with `count` titled attendants, in the order given.
fn state_with_attendants(count: usize) -> jinn_kernel::AppState {
    let mut state = jinn_kernel::AppState::default_with_scope_focus();
    let parent = jinn_session_state::ChatSessionState::new();
    let parent_id = parent.session_id().clone();
    state.session.insert(parent);
    for name in ["alpha", "beta", "gamma"].iter().take(count) {
        let mut attendant = jinn_session_state::ChatSessionState::new_attendant(
            state.session.get(&parent_id).expect("parent"),
            true,
        );
        attendant.set_title((*name).to_owned());
        state.session.insert(attendant);
    }
    state.session.set_active(parent_id);
    state
}

/// Dispatches a sidebar row by (scope, action) and applies its scope signal
/// the way the kernel's handler does — the opener's whole effect is a
/// `ScopeSignal::Push`, which nothing dispatches on its own.
fn press(
    routes: &KeyRoutes,
    state: &mut jinn_kernel::AppState,
    scope: jinn_slices::SliceScopeId,
    action: &'static str,
) {
    let slices = jinn_slices::Slices::new();
    let result = routes
        .action_for(
            &jinn_slices::DynamicIntent::new(scope, action, action),
            ActionCtx {
                state,
                slices: &slices,
                config: jinn_slices::empty_config_layer(),
                key_bytes: Vec::new(),
            },
        )
        .unwrap_or_else(|| panic!("the sidebar binds {action:?}"));
    match result.scope_signal {
        Some(jinn_slices::ScopeSignal::Push(pushed)) => state
            .frontend
            .scope_push(jinn_slices::FocusScope::Dynamic(pushed)),
        Some(jinn_slices::ScopeSignal::PopIf(_)) | None => {}
    }
}

#[rstest::rstest]
fn P_in_the_attendants_section_opens_the_properties_popup() {
    // Given the attendants section focused over one attendant.
    let mut state = state_with_attendants(1);
    focus_section(&state, jinn_sidebar_msg::SidebarSectionId::Attendant);
    state
        .frontend
        .update_sections(|s| s.attendant.selected_index = Some(0));

    // When `P` is pressed in the sidebar's attendants section.
    let routes = KeyRoutes::new();
    attach_sidebar_rows(&routes);
    press(
        &routes,
        &mut state,
        jinn_sidebar_msg::SidebarSectionId::Attendant.scope_id(),
        "attendant-open-properties",
    );

    // Then the properties popup is open.
    assert_eq!(
        state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(jinn_attendant_msg::attendant_properties_scope()),
        "`P` must open the properties popup from the attendants section"
    );
}

#[rstest::rstest]
fn P_in_the_attendants_section_opens_the_highlighted_attendant() {
    // Given two attendants under the active session, with the cursor on the
    // second.
    let mut state = state_with_attendants(2);
    focus_section(&state, jinn_sidebar_msg::SidebarSectionId::Attendant);
    state
        .frontend
        .update_sections(|s| s.attendant.selected_index = Some(1));
    let expected = jinn_attendant::section_rows::attendant_rows(&state)[1]
        .session_id
        .clone();
    let properties = properties_cell(&state);

    // When `P` is pressed.
    let routes = KeyRoutes::new();
    attach_sidebar_rows(&routes);
    press(
        &routes,
        &mut state,
        jinn_sidebar_msg::SidebarSectionId::Attendant.scope_id(),
        "attendant-open-properties",
    );

    // Then the popup is seeded for the row under the cursor, not the first
    // one — the two sections index different lists, so a handler reading the
    // wrong one would open a different attendant's properties.
    assert_eq!(
        properties.read().session_id.as_ref(),
        Some(&expected),
        "`P` must open the highlighted attendant"
    );
}

#[rstest::rstest]
fn P_in_the_attendants_section_over_no_attendants_opens_nothing() {
    // Given a session with no attendants, so the section has no rows to
    // highlight and the cursor it holds is stale.
    let mut state = state_with_attendants(0);
    focus_section(&state, jinn_sidebar_msg::SidebarSectionId::Attendant);
    state
        .frontend
        .update_sections(|s| s.attendant.selected_index = Some(0));
    let before = state.frontend.scope();

    // When `P` is pressed.
    let routes = KeyRoutes::new();
    attach_sidebar_rows(&routes);
    press(
        &routes,
        &mut state,
        jinn_sidebar_msg::SidebarSectionId::Attendant.scope_id(),
        "attendant-open-properties",
    );

    // Then nothing opened — there is no attendant to edit.
    assert_eq!(state.frontend.scope(), before);
}

#[rstest::rstest]
fn P_in_the_sessions_section_on_a_user_session_opens_nothing() {
    // Given the sessions section focused on a plain user session.
    let mut state = state_with_attendants(0);
    focus_section(&state, jinn_sidebar_msg::SidebarSectionId::Sessions);
    state
        .frontend
        .update_sections(|s| s.sessions.selected_index = Some(0));
    let before = state.frontend.scope();

    // When `P` is pressed.
    let routes = KeyRoutes::new();
    attach_sidebar_rows(&routes);
    press(
        &routes,
        &mut state,
        jinn_sidebar_msg::SidebarSectionId::Sessions.scope_id(),
        "session-attendant-properties",
    );

    // Then nothing opened. The popup has no field to edit on a user
    // session, and the sessions section reaches users as well as
    // attendants.
    assert_eq!(state.frontend.scope(), before);
}
