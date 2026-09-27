//! Activates the session under the cursor.

use jinn_domain::common::app_state::AppState;

use crate::sections::sessions::state::sorted_open_sessions;
use jinn_chat_log_view::kernel_element::activate_session;
use jinn_domain::protocol::IntentResult;

/// Activates the session under the cursor.
///
/// Called when the user presses Enter in the sessions section.
/// Uses `swap_base` to replace the entire scope stack, effectively
/// closing the sidebar and switching to the target view.
/// - For session entries: swaps to Normal (chat view). No re-scan commands
///   are emitted: each session's discovered skills/prompts/context-files
///   are ephemeral and persist across activation changes, and were
///   hydrated when the session was created/loaded.
pub fn handle_session_activate(state: &mut AppState) -> IntentResult {
    activate_selected(state, false)
}

/// Activates the session under the cursor and enters Insert mode.
///
/// Called when the user presses `i` in the sessions section. Like
/// [`handle_session_activate`] but lands in Input mode instead of Normal,
/// so the user can immediately start typing. The scope stack ends up
/// `[Normal, Input]` — Normal as the base so that ESC (`clear_overlays`)
/// correctly returns to Normal, with Input on top as the active mode.
/// - For session entries: activates the session, swaps to Normal as the
///   base, then pushes Input.
pub fn handle_session_activate_insert(state: &mut AppState) -> IntentResult {
    activate_selected(state, true)
}

/// Switches to the session under the cursor, measuring it if it needs it.
///
/// Every session the sidebar lists is already in memory, so activation never
/// reads from disk. It can still be slow: a session whose line counts are not
/// cached makes the next frame lay out its whole history inline, which is what
/// freezes the UI on a large session. So a session that is not yet measured is
/// switched to behind the loading indication, and measured off the render
/// thread — the same hand-off a session restored from the archive gets.
///
/// `enter_input` selects the scope the sidebar leaves behind: Normal for a
/// plain activation, Input when the user asked to start typing.
fn activate_selected(state: &mut AppState, enter_input: bool) -> IntentResult {
    use jinn_slices::FocusScope;

    if !matches!(
        state.frontend.sidebar_section(),
        Some(jinn_sidebar_msg::SidebarSectionId::Sessions)
    ) {
        return IntentResult::empty();
    }
    let Some(index) = state
        .frontend
        .with_sections(|s| s.sessions.selected_index, || None)
    else {
        return IntentResult::empty();
    };
    let sessions = sorted_open_sessions(state);
    let Some(entry) = sessions.get(index) else {
        return IntentResult::empty();
    };
    let target_id = entry.id.clone();

    state.frontend.scope_swap_base(FocusScope::Normal);
    if enter_input {
        state.frontend.scope_push(FocusScope::Input);
    }
    activate_session(state, target_id, IntentResult::empty())
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
    use jinn_domain::common::app_state::AppState;
    use jinn_session_store_msg::SessionLoadRequested;
    use jinn_slices::FocusScope;

    use jinn_core_types::SessionId;

    /// Two sessions exist in state; cursor points at the second-inserted
    /// session.
    ///
    /// The second session holds history: an empty one has no lines to lay out,
    /// so it is always already measured and would never exercise the
    /// measurement path.
    fn state_with_two_sessions_cursor_on_second() -> (AppState, SessionId) {
        let mut state = AppState::default_with_scope_focus();
        let _first = state.session.active_session_id().clone();
        let mut second_session = jinn_session_state::ChatSessionState::default();
        second_session.push_entry(jinn_core_types::ChatEntry::user("an earlier question"));
        second_session.push_entry(jinn_core_types::ChatEntry::assistant("an earlier answer"));
        let second = second_session.session_id().clone();
        state.session.insert(second_session);
        // Cursor points at the second session in sorted order.
        let sessions = sorted_open_sessions(&state);
        let target_idx = sessions
            .iter()
            .position(|e| e.id == second)
            .expect("second session present");
        state
            .frontend
            .update_sections(|s| s.sessions.selected_index = Some(target_idx));
        state
            .frontend
            .scope_push(jinn_sidebar_msg::SidebarSectionId::Sessions.focus_scope());
        (state, second)
    }

    /// Leaves the cache warm for the session under the cursor, as real chat log
    /// frames at `width` would.
    ///
    /// Renders rather than inserting counts by hand, so the cache is filled
    /// with exactly the keys the production render pass uses — hand-built
    /// counts would test the coverage query against the helper instead of
    /// against the renderer.
    ///
    /// The cursor's session is rendered *and* the session that was on screen
    /// before, because the activation reads its width: the switch is only
    /// considered free if the target's counts match the width the next frame
    /// will use, and the on-screen session is the one that knows it.
    fn with_measured_active_session(state: &mut AppState, width: u16) {
        use jinn_chat_log_view::kernel_element::ChatLogElement;
        use jinn_domain::common::render_ctx::RenderCtx;
        use jinn_domain::common::ui_element::UiElement;
        use jinn_testutil::setup_term;

        let target_id = {
            let index = state
                .frontend
                .with_sections(|s| s.sessions.selected_index, || None)
                .expect("the fixture puts the cursor on a session");
            sorted_open_sessions(state)[index].id.clone()
        };
        let on_screen = state.session.active_session_id().clone();
        let (mut terminal, area) = setup_term(width, 10);
        let mut render_once = |session_id: &SessionId| {
            state.session.set_active(session_id.clone());
            terminal
                .draw(|frame| {
                    let slices = jinn_slices::Slices::new();
                    let overlay_views = jinn_slices::OverlayViews::new();
                    let mut element = ChatLogElement::new();
                    let ctx = RenderCtx::new(
                        state,
                        &slices,
                        &overlay_views,
                        jinn_config::empty_config_layer(),
                    );
                    element.render(frame, area, &ctx);
                })
                .expect("measure a session");
        };

        render_once(&target_id);
        render_once(&on_screen);
        // The on-screen session is active again, as the user left it.
        assert_eq!(
            state.session.active_session_id(),
            &on_screen,
            "the helper must leave the on-screen session unchanged"
        );
    }

    #[rstest::rstest]
    fn activate_session_switches_active_session_and_emits_no_commands() {
        // Given a sessions sidebar with cursor on a measured session.
        let (mut state, expected_id) = state_with_two_sessions_cursor_on_second();
        with_measured_active_session(&mut state, 60);

        // When activating.
        let result = handle_session_activate(&mut state);

        // Then the active session is now the one under the cursor.
        assert_eq!(state.session.active_session_id(), &expected_id);
        // And no commands are emitted: a measured session needs no
        // measurement, and skills/prompts/context-files are ephemeral and
        // were hydrated when the session was created/loaded.
        assert!(result.message_names.is_empty());
    }

    #[rstest::rstest]
    fn activate_session_with_no_cursor_emits_nothing() {
        // Given sessions sidebar but no selected index.
        let mut state = AppState::default_with_scope_focus();
        state
            .frontend
            .scope_push(jinn_sidebar_msg::SidebarSectionId::Sessions.focus_scope());
        // selected_index stays None.

        // When activating.
        let result = handle_session_activate(&mut state);

        // Then no commands emitted.
        assert!(result.message_names.is_empty());
    }

    #[rstest::rstest]
    fn activate_outside_sessions_section_emits_nothing() {
        // Given Normal scope (not sessions sidebar).
        let mut state = AppState::default_with_scope_focus();

        // When activating.
        let result = handle_session_activate(&mut state);

        // Then no commands emitted.
        assert!(result.message_names.is_empty());
    }

    #[rstest::rstest]
    fn activate_unmeasured_session_arms_the_load_guard() {
        // Given a sessions sidebar with cursor on a session that has never
        // been measured.
        let (mut state, _expected_id) = state_with_two_sessions_cursor_on_second();

        // When activating.
        handle_session_activate(&mut state);

        // Then the loading indication stands, so the next frame does not lay
        // the whole history out inline.
        assert!(
            state.session.is_loading(),
            "an unmeasured session must be measured before it is drawn"
        );
    }

    #[rstest::rstest]
    fn activate_unmeasured_session_requests_a_measurement() {
        // Given a sessions sidebar with cursor on an unmeasured session.
        let (mut state, _expected_id) = state_with_two_sessions_cursor_on_second();

        // When activating.
        let result = handle_session_activate(&mut state);

        // Then a measurement is requested for it.
        assert_eq!(
            result.message_names,
            vec!["SessionLoadRequested".to_owned()],
            "the session must be measured off the render thread"
        );
    }

    #[rstest::rstest]
    fn activate_measured_session_arms_no_load_guard() {
        // Given a sessions sidebar with cursor on an already-measured session.
        let (mut state, _expected_id) = state_with_two_sessions_cursor_on_second();
        with_measured_active_session(&mut state, 60);

        // When activating.
        handle_session_activate(&mut state);

        // Then no indication appears: the switch is immediate.
        assert!(
            !state.session.is_loading(),
            "a measured session must switch without an indication"
        );
    }

    #[rstest::rstest]
    fn activate_measured_session_requests_no_measurement() {
        // Given a sessions sidebar with cursor on an already-measured session.
        let (mut state, _expected_id) = state_with_two_sessions_cursor_on_second();
        with_measured_active_session(&mut state, 60);

        // When activating.
        let result = handle_session_activate(&mut state);

        // Then nothing is dispatched.
        assert!(
            result.message_names.is_empty(),
            "an already-measured session must not be measured again"
        );
    }

    #[rstest::rstest]
    fn activate_session_measured_at_another_width_is_measured_again() {
        // Given a session measured at a width the chat log no longer uses.
        let (mut state, _expected_id) = state_with_two_sessions_cursor_on_second();
        with_measured_active_session(&mut state, 60);

        // When activating, with the on-screen session's width changed since
        // that measurement.
        state.active_session_mut().set_content_width(90);
        let result = handle_session_activate(&mut state);

        // Then it is measured again, because those counts are wrong at the
        // width the next frame will use.
        assert_eq!(
            result.message_names,
            vec!["SessionLoadRequested".to_owned()]
        );
    }

    #[rstest::rstest]
    fn activate_insert_switches_active_session() {
        // Given a sessions sidebar with cursor on a non-active session.
        let (mut state, expected_id) = state_with_two_sessions_cursor_on_second();

        // When activating into insert mode.
        handle_session_activate_insert(&mut state);

        // Then the active session is now the one under the cursor.
        assert_eq!(state.session.active_session_id(), &expected_id);
    }

    #[rstest::rstest]
    fn activate_insert_pushes_input_with_normal_base() {
        // Given a sessions sidebar with cursor on a session.
        let (mut state, _expected_id) = state_with_two_sessions_cursor_on_second();

        // When activating into insert mode.
        handle_session_activate_insert(&mut state);

        // Then the top of the stack is Input (insert mode).
        assert_eq!(state.frontend.scope(), FocusScope::Input);
        // And the base is Normal so ESC can return there via clear_overlays.
        assert_eq!(state.frontend.scope_parent(), Some(FocusScope::Normal));
    }

    #[rstest::rstest]
    fn activate_insert_arms_the_load_guard_for_an_unmeasured_session() {
        // Given a sessions sidebar with cursor on an unmeasured session.
        let (mut state, _expected_id) = state_with_two_sessions_cursor_on_second();

        // When activating into insert mode.
        handle_session_activate_insert(&mut state);

        // Then the session is still measured before it is drawn.
        assert!(state.session.is_loading());
    }

    #[rstest::rstest]
    fn activate_insert_outside_sessions_section_is_noop() {
        // Given Normal scope (not sessions sidebar).
        let mut state = AppState::default_with_scope_focus();
        let initial_scope = state.frontend.scope().clone();

        // When activating into insert mode.
        handle_session_activate_insert(&mut state);

        // Then the scope is unchanged.
        assert_eq!(state.frontend.scope(), initial_scope);
    }

    #[rstest::rstest]
    fn activate_insert_with_no_selected_index_is_noop() {
        // Given sessions sidebar but no selected index.
        let mut state = AppState::default_with_scope_focus();
        state
            .frontend
            .scope_push(jinn_sidebar_msg::SidebarSectionId::Sessions.focus_scope());
        let initial_scope = state.frontend.scope().clone();

        // When activating into insert mode.
        handle_session_activate_insert(&mut state);

        // Then the scope is unchanged.
        assert_eq!(state.frontend.scope(), initial_scope);
    }

    use jinn_slices::route_publish::PublishSink;
    use std::sync::Mutex;

    /// A publish sink that keeps every payload published through it.
    #[derive(Default)]
    struct RecordingSink {
        published: Mutex<Vec<(String, serde_json::Value)>>,
    }

    impl PublishSink for RecordingSink {
        fn publish_schema(
            &self,
            schema_id: trouper::schema::SchemaId,
            payload: serde_json::Value,
            name: &'static str,
        ) {
            self.published
                .lock()
                .expect("sink lock")
                .push((format!("{schema_id}"), payload));
            let _ = name;
        }
    }

    /// The activation request a result publishes, as the store actor will
    /// decode it.
    fn load_request(result: IntentResult) -> Option<SessionLoadRequested> {
        let sink = RecordingSink::default();
        for closure in result.messages {
            closure(&sink);
        }
        let published = sink.published.lock().expect("sink lock");
        let (_, payload) = published
            .iter()
            .find(|(id, _)| id.ends_with("SessionLoadRequested"))
            .expect("an activation request is published");
        Some(serde_json::from_value(payload.clone()).expect("decodes"))
    }

    #[rstest::rstest]
    #[test]
    fn the_activation_request_carries_the_width_the_chat_log_is_using() {
        // Given two sessions, the second unmeasured, the first having last
        // rendered at 72 columns.
        let (mut state, second) = state_with_two_sessions_cursor_on_second();
        state.active_session_mut().set_content_width(72);

        // When the sidebar activates the unmeasured session.
        let result = handle_session_activate(&mut state);

        // Then the request travels with that width.
        //
        // The activation has already switched sessions by this point, so an
        // actor that re-read the width would find the target's never-rendered
        // zero and measure the whole history at a width nothing renders at.
        let request = load_request(result);
        assert_eq!(
            request.as_ref().and_then(|r| r.content_width),
            Some(72),
            "the request must carry the width read before the switch"
        );
        assert_eq!(request.map(|r| r.session_id), Some(second));
    }

    #[rstest::rstest]
    #[test]
    fn a_measured_session_asks_for_nothing_at_all() {
        // Given two sessions with the second already measured at this width.
        let (mut state, second) = state_with_two_sessions_cursor_on_second();
        state.active_session_mut().set_content_width(72);
        with_measured_active_session(&mut state, 72);

        // When the sidebar activates it.
        let result = handle_session_activate(&mut state);

        // Then nothing is published, so no spinner can appear.
        let sink = RecordingSink::default();
        for closure in result.messages {
            closure(&sink);
        }
        assert!(
            sink.published.lock().expect("sink lock").is_empty(),
            "a measured session must not be re-measured"
        );
        assert_eq!(state.session.active_session_id(), &second);
    }
}
