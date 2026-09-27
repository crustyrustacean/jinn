// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! The tool picker's observable behavior.
//!
//! Tests that assert *wiring* run the slice's real `activate_picker`, so a
//! picker that registered its cell but forgot its rows or its input hook fails
//! loudly rather than passing against a hand-built stand-in. Tests that assert
//! *behavior* drive the picker's public entry points.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test module, panics are acceptable"
)]

use jinn_slices::cell::TypedCell;
use jinn_slices::route::{EditIntent, RouteOutcome, ScopeSignal};
use jinn_slices::{KeyRoutes, SliceHost, Slices};
use jinn_tools_msg::{ToolPickerState, tool_picker_scope};

/// A tool definition in the shape the tools registry holds.
struct Def {
    name: &'static str,
    description: &'static str,
    provider_gated: bool,
}

impl Def {
    /// A tool any provider can run.
    fn plain(name: &'static str, description: &'static str) -> Self {
        Self {
            name,
            description,
            provider_gated: false,
        }
    }

    /// A tool that only an OpenRouter model may run, standing in for the
    /// web-search tool the real registry gates by provider.
    fn openrouter_only(name: &'static str, description: &'static str) -> Self {
        Self {
            name,
            description,
            provider_gated: true,
        }
    }

    /// The registry `ToolDefinition` this fixture stands for.
    fn definition(&self) -> jinn_core_types::ToolDefinition {
        jinn_core_types::ToolDefinition {
            name: self.name.to_owned(),
            description: self.description.to_owned(),
            parameters: serde_json::json!({}),
            prompt_snippet: None,
            prompt_guidelines: vec![],
            server_tool_type: self
                .provider_gated
                .then_some(jinn_core_types::ServerToolType::OpenrouterWebSearch),
        }
    }
}

/// The slice wired exactly as composition wires it.
struct Wired {
    slices: Slices,
    routes: KeyRoutes,
    state: std::cell::RefCell<jinn_domain::AppState>,
}

impl Wired {
    /// Builds the slice with `defs` already registered in the tools registry
    /// cell, so the tests describe the picker rather than the tool
    /// orchestrator.
    async fn new(defs: Vec<Def>) -> Self {
        let slices = Slices::new();
        let mut viewport = jinn_slices::view::Viewport::new();
        let overlay_views = jinn_slices::OverlayViews::new();
        let routes = KeyRoutes::new();
        let services = jinn_domain::Services::new_fake().await;
        {
            let mut host = SliceHost::new(
                &slices,
                &mut viewport,
                &overlay_views,
                &routes,
                &services.trouper_system,
            );
            crate::activate_picker(&mut host);
        }
        // The picker pushes a real scope, so the app state needs the
        // scope-focus cell its facade writes through.
        slices
            .register(
                jinn_slices::scope_focus_slot(),
                jinn_slices::ScopeFocusState::default(),
            )
            .expect("scope-focus cell is not registered yet");
        // The tools registry cell is what the picker reads its rows from;
        // `activate` is the only thing that normally registers it.
        slices
            .register(
                jinn_tools_msg::tools_registry_slot(),
                jinn_tools_msg::ToolRegistry::default(),
            )
            .expect("tools registry cell is not registered yet");
        slices
            .reader::<jinn_tools_msg::ToolRegistry>(&jinn_tools_msg::tools_registry_slot())
            .expect("tools registry cell is registered")
            .update(|registry| {
                for def in &defs {
                    registry
                        .global
                        .insert(def.name.to_owned(), def.definition());
                }
            });

        // `AppState::default()` (not `default_with_scope_focus`) so the
        // `attach_slices` below is the first and only attachment: the facade
        // handle is a `OnceLock`, and the test harness already minted the
        // cells on *this* `Slices` above.
        let mut state = jinn_domain::AppState::default();
        let origin = jinn_session_state::ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state.frontend.attach_slices(slices.clone());
        state.session.attach_slices(slices.clone());
        Self {
            slices,
            routes,
            state: std::cell::RefCell::new(state),
        }
    }

    /// The picker cell.
    fn cell(&self) -> TypedCell<ToolPickerState> {
        self.slices
            .reader(&jinn_tools_msg::tool_picker_slot())
            .expect("the picker registers its cell at activation")
    }

    /// The names the filter currently shows, in display order.
    ///
    /// Reads the *filtered* view, not the underlying item list: a picker whose
    /// filter text is written but never applied still has every item present.
    fn visible_names(&self) -> Vec<String> {
        let state = self.cell();
        let guard = state.read();
        (0..guard.selection.filtered_count())
            .filter_map(|i| guard.selection.filtered_item(i))
            .map(|item| item.entry().name.clone())
            .collect()
    }

    /// The index of the highlighted row.
    fn highlighted(&self) -> usize {
        self.cell().read().selection.selection()
    }

    /// The filter text.
    fn filter(&self) -> String {
        self.cell().read().selection.filter().to_owned()
    }

    /// Whether the row named `name` currently renders enabled.
    fn enabled(&self, name: &str) -> Option<bool> {
        let state = self.cell();
        let guard = state.read();
        guard
            .selection
            .items()
            .iter()
            .find(|item| item.entry().name == name)
            .map(|item| item.entry().enabled)
    }

    /// The session's live disabled-tool set.
    fn session_disabled(&self) -> std::collections::HashSet<String> {
        self.state
            .borrow()
            .active_session()
            .disabled_tools()
            .clone()
    }

    /// Opens the picker through its real `open` route action.
    fn open(&self) -> jinn_slices::RouteResult {
        self.fire("open-tool-picker")
    }

    /// Dispatches one of the picker's actions by its route action name.
    fn fire(&self, action: &str) -> jinn_slices::RouteResult {
        let mut state = self.state.borrow_mut();
        self.routes
            .action_for(
                &jinn_slices::DynamicIntent::new(tool_picker_scope(), action, action),
                jinn_slices::ActionCtx {
                    state: &mut *state,
                    slices: &self.slices,
                    config: jinn_slices::empty_config_layer(),
                    key_bytes: Vec::new(),
                },
            )
            .unwrap_or_else(|| panic!("the tool picker attaches an action named {action}"))
    }

    /// Feeds an edit intent to the picker's registered input hook.
    fn edit(&self, intent: &EditIntent) {
        let hook = self
            .routes
            .input_hook(&tool_picker_scope())
            .expect("the picker registers an input hook for its filter");
        hook(intent).expect("the filter hook always consumes the edit");
    }

    /// Draws one frame of the picker at `100x30` and returns the buffer's
    /// symbols, row by row.
    fn draw(&self) -> Vec<String> {
        draw_frame(self, ratatui::layout::Rect::new(0, 0, 100, 30))
    }
}

/// Draws one frame of the picker's popup into a fresh test terminal and
/// returns the buffer's symbols, row by row.
fn draw_frame(wired: &Wired, area: ratatui::layout::Rect) -> Vec<String> {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");
    let facts =
        jinn_slices::RenderFacts::new(wired.state.borrow().frontend.theme.clone(), &wired.slices);
    terminal
        .draw(|frame| {
            let popup = crate::tool_picker_render::tool_picker_overlay_rect(&area)
                .expect("geometry fn yields a popup rect");
            crate::tool_picker_render::render_tool_picker(frame, popup, &facts);
        })
        .expect("draw");
    terminal
        .backend()
        .buffer()
        .content
        .chunks(area.width as usize)
        .map(|row| row.iter().map(ratatui::buffer::Cell::symbol).collect())
        .collect()
}

/// The three tools most tests need.
fn three_tools() -> Vec<Def> {
    vec![
        Def::plain("edit", "Edit files"),
        Def::plain("Bash", "Run shell"),
        Def::plain("read", "Read files"),
    ]
}

// ── 1. Opens with entries ───────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn opening_the_picker_shows_the_sessions_tools() {
    // Given a session with three registered tools.
    let wired = Wired::new(three_tools()).await;

    // When the picker is opened.
    wired.open();

    // Then every tool has a row.
    assert_eq!(wired.visible_names(), vec!["Bash", "edit", "read"]);
}

#[rstest::rstest]
#[tokio::test]
async fn opening_the_picker_sorts_tools_case_insensitively_by_name() {
    // Given a session whose tools are registered in registry hash order.
    let wired = Wired::new(three_tools()).await;

    // When the picker is opened.
    wired.open();

    // Then the rows are in case-insensitive name order, not insertion order.
    assert_eq!(wired.visible_names(), vec!["Bash", "edit", "read"]);
}

#[rstest::rstest]
#[tokio::test]
async fn opening_the_picker_starts_with_the_highlight_on_the_first_row() {
    // Given a session with two tools.
    let wired = Wired::new(vec![Def::plain("a", "first"), Def::plain("b", "second")]).await;

    // When the picker is opened.
    wired.open();

    // Then the cursor sits on the first row.
    assert_eq!(wired.highlighted(), 0);
}

#[rstest::rstest]
#[tokio::test]
async fn opening_the_picker_pushes_its_own_scope() {
    // Given a session with one tool.
    let wired = Wired::new(vec![Def::plain("bash", "Run shell")]).await;

    // When the picker is opened.
    wired.open();

    // Then the picker's dynamic scope is on top of the stack.
    let scope = wired.state.borrow().frontend.scope();
    assert_eq!(
        scope,
        jinn_slices::FocusScope::Dynamic(tool_picker_scope()),
        "opening the picker must push its own dynamic scope"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn a_pre_disabled_tool_renders_off_when_the_picker_opens() {
    // Given a session whose profile already disables one tool.
    let wired = Wired::new(three_tools()).await;
    wired
        .state
        .borrow_mut()
        .active_session_mut()
        .set_disabled_tools(["read"].iter().map(|s| (*s).to_owned()).collect());

    // When the picker is opened.
    wired.open();

    // Then that tool's row shows the off marker and the others show on.
    assert_eq!(wired.enabled("read"), Some(false));
    assert_eq!(wired.enabled("edit"), Some(true));
}

#[rstest::rstest]
#[tokio::test]
async fn a_subagent_stamped_tool_renders_off_when_the_picker_opens() {
    // Given a subagent session, whose spawn stamp suppresses the task tool.
    let wired = Wired::new(vec![Def::plain(
        jinn_tools_msg::TASK_TOOL_NAME,
        "Delegate a sub-task",
    )])
    .await;
    wired
        .state
        .borrow_mut()
        .active_session_mut()
        .set_disabled_tools(
            [jinn_tools_msg::TASK_TOOL_NAME.to_owned()]
                .into_iter()
                .collect(),
        );

    // When the picker is opened.
    wired.open();

    // Then the task tool's row renders disabled, so the stamp is visible.
    assert_eq!(wired.enabled(jinn_tools_msg::TASK_TOOL_NAME), Some(false));
}

// ── 2. The filter narrows ───────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn typing_a_character_filters_the_list() {
    // Given an open picker over three tools.
    let wired = Wired::new(three_tools()).await;
    wired.open();

    // When the user types "read".
    for ch in "read".chars() {
        wired.edit(&EditIntent::InsertChar(ch));
    }

    // Then the filter holds the text and only the matching row survives.
    assert_eq!(wired.filter(), "read");
    assert_eq!(wired.visible_names(), vec!["read"]);
}

#[rstest::rstest]
#[tokio::test]
async fn the_filter_also_matches_a_tools_description() {
    // Given an open picker whose second tool's description mentions "shell".
    let wired = Wired::new(three_tools()).await;
    wired.open();

    // When the user types "shell".
    for ch in "shell".chars() {
        wired.edit(&EditIntent::InsertChar(ch));
    }

    // Then the row matched on its description alone survives, and the rows
    // whose name and description both miss the term are gone.
    assert_eq!(wired.visible_names(), vec!["Bash"]);
}

#[rstest::rstest]
#[tokio::test]
async fn backspace_shortens_the_filter() {
    // Given an open picker whose filter reads "read".
    let wired = Wired::new(three_tools()).await;
    wired.open();
    for ch in "read".chars() {
        wired.edit(&EditIntent::InsertChar(ch));
    }

    // When backspace is pressed twice.
    wired.edit(&EditIntent::DeleteBackward);
    wired.edit(&EditIntent::DeleteBackward);

    // Then the filter reads "re" — narrower than the full name, so the rows
    // that only fuzzy-match the short term come back alongside it.
    assert_eq!(wired.filter(), "re");
    assert_eq!(wired.visible_names(), vec!["read", "Bash"]);
}

#[rstest::rstest]
#[tokio::test]
async fn a_provider_gated_tool_is_hidden_from_a_direct_provider() {
    // Given a session on a non-OpenRouter model with a provider-gated tool
    // registered alongside an ordinary one.
    let wired = Wired::new(vec![
        Def::openrouter_only("openrouter:web_search", "Search the web"),
        Def::plain("read", "Read files"),
    ])
    .await;
    wired.state.borrow_mut().active_session_mut().set_model(
        jinn_core_types::model_selection::ModelSelection::Single("zai/glm-4.6".to_owned()),
    );

    // When the picker is opened.
    wired.open();

    // Then only the tool this provider can actually run is offered.
    assert_eq!(wired.visible_names(), vec!["read"]);
}

#[rstest::rstest]
#[tokio::test]
async fn a_provider_gated_tool_is_offered_to_an_openrouter_model() {
    // Given a session on an OpenRouter model with a provider-gated tool.
    let wired = Wired::new(vec![
        Def::openrouter_only("openrouter:web_search", "Search the web"),
        Def::plain("read", "Read files"),
    ])
    .await;
    wired.state.borrow_mut().active_session_mut().set_model(
        jinn_core_types::model_selection::ModelSelection::Single(
            "openrouter/openai/gpt-oss-120b".to_owned(),
        ),
    );

    // When the picker is opened.
    wired.open();

    // Then the gated tool is offered too.
    assert_eq!(wired.visible_names(), vec!["openrouter:web_search", "read"]);
}

// ── 3. Navigation ───────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn down_arrow_moves_the_highlight() {
    // Given an open picker whose highlight is on the first row.
    let wired = Wired::new(three_tools()).await;
    wired.open();

    // When the down arrow is pressed.
    wired.fire("move-tool-picker-down");

    // Then the highlight is on the second row.
    assert_eq!(wired.highlighted(), 1);
}

#[rstest::rstest]
#[tokio::test]
async fn up_arrow_stops_at_the_first_row() {
    // Given an open picker whose highlight is already at the top.
    let wired = Wired::new(three_tools()).await;
    wired.open();

    // When the up arrow is pressed.
    wired.fire("move-tool-picker-up");

    // Then the highlight does not wrap or move past the first row.
    assert_eq!(wired.highlighted(), 0);
}

#[rstest::rstest]
#[tokio::test]
async fn page_down_moves_the_highlight_past_the_first_row() {
    // Given an open picker with more rows than fit on one page.
    let many: Vec<Def> = (0..60)
        .map(|i| Def::plain_owned(format!("t{i:02}"), "A tool".to_owned()))
        .collect();
    let wired = Wired::new(many).await;
    wired.open();

    // When page down is pressed.
    wired.fire("page-tool-picker-down");

    // Then the highlight left the first row.
    assert!(
        wired.highlighted() > 1,
        "page down must advance past the first row, got {}",
        wired.highlighted()
    );
}

#[rstest::rstest]
#[tokio::test]
async fn page_down_steps_by_the_rows_the_last_frame_actually_laid_out() {
    // Given an open picker with far more rows than a frame can show, and a
    // frame drawn so the cell learns how many rows are on screen.
    let many: Vec<Def> = (0..200)
        .map(|i| Def::plain_owned(format!("t{i:03}"), "A tool".to_owned()))
        .collect();
    let wired = Wired::new(many).await;
    wired.open();
    // A short frame: the laid-out list pane holds far fewer rows than the
    // pre-render fallback, so a picker that paged by the fallback would move a
    // visibly different distance.
    let frame = ratatui::layout::Rect::new(0, 0, 100, 16);
    draw_frame(&wired, frame);
    let on_screen = crate::tool_picker_viewport::results_viewport(
        crate::tool_picker_render::tool_picker_overlay_rect(&frame)
            .expect("geometry fn yields a popup rect"),
    );

    // When page down is pressed.
    wired.fire("page-tool-picker-down");

    // Then the highlight advanced by half a screen of rows — a page of what
    // the user can see, not of a fixed guess about how much that is.
    assert_ne!(
        on_screen,
        jinn_tools_msg::TOOL_PICKER_RESULTS_VIEWPORT_FALLBACK
    );
    assert_eq!(wired.highlighted(), (on_screen / 2).max(1));
}

// ── 4. Confirm ──────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn confirming_asks_the_scope_stack_to_pop_the_picker() {
    // Given an open picker.
    let wired = Wired::new(three_tools()).await;
    wired.open();

    // When enter is pressed.
    let result = wired.fire("confirm-tool-picker");

    // Then the result pops the picker's own scope, closing the menu.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(tool_picker_scope()))
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirming_writes_the_toggled_off_tools_to_the_session() {
    // Given an open picker whose first row was toggled off.
    let wired = Wired::new(three_tools()).await;
    wired.open();
    wired.fire("toggle-highlighted-tool");

    // When enter is pressed.
    wired.fire("confirm-tool-picker");

    // Then the toggled-off tool lands in the session's disabled set.
    assert!(
        wired.session_disabled().contains("Bash"),
        "confirm must write the toggled set to the session; got {:?}",
        wired.session_disabled()
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirming_clears_the_revert_snapshot() {
    // Given an open picker whose highlight has been toggled.
    let wired = Wired::new(three_tools()).await;
    wired.open();
    wired.fire("toggle-highlighted-tool");

    // When enter is pressed.
    wired.fire("confirm-tool-picker");

    // Then nothing is left for a later close to restore — the choice is
    // authoritative and must survive the picker going away.
    let cell = wired.cell();
    assert!(
        cell.read().snapshot.is_none(),
        "confirm must clear the snapshot so nothing reverts the choice"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn toggling_alone_never_writes_the_session_disabled_set() {
    // Given an open picker over three tools.
    let wired = Wired::new(three_tools()).await;
    wired.open();
    assert!(wired.session_disabled().is_empty());

    // When a row is toggled off.
    wired.fire("toggle-highlighted-tool");

    // Then the session's disabled set is still empty: toggling edits the menu,
    // only confirming commits.
    assert!(
        wired.session_disabled().is_empty(),
        "a toggle must not write the disabled set; got {:?}",
        wired.session_disabled()
    );
}

#[rstest::rstest]
#[tokio::test]
async fn escaping_never_writes_the_session_disabled_set() {
    // Given an open picker whose row was toggled off.
    let wired = Wired::new(three_tools()).await;
    wired.open();
    wired.fire("toggle-highlighted-tool");

    // When escape is pressed.
    wired.fire("cancel-tool-picker");

    // Then the session's disabled set is untouched.
    assert!(
        wired.session_disabled().is_empty(),
        "escape must restore, never commit; got {:?}",
        wired.session_disabled()
    );
}

// ── 5. Escape ───────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn escape_restores_the_tools_that_were_disabled_when_the_picker_opened() {
    // Given a session with one tool already disabled, and a picker that has
    // since toggled a second one off.
    let wired = Wired::new(three_tools()).await;
    wired
        .state
        .borrow_mut()
        .active_session_mut()
        .set_disabled_tools(["read"].iter().map(|s| (*s).to_owned()).collect());
    wired.open();
    wired.fire("toggle-highlighted-tool");

    // When escape is pressed.
    wired.fire("cancel-tool-picker");

    // Then the pre-open disabled set is back and the toggle never landed.
    assert_eq!(
        wired.session_disabled(),
        ["read".to_owned()].into_iter().collect()
    );
}

#[rstest::rstest]
#[tokio::test]
async fn escape_asks_the_scope_stack_to_pop_the_picker() {
    // Given an open picker.
    let wired = Wired::new(three_tools()).await;
    wired.open();

    // When escape is pressed.
    let result = wired.fire("cancel-tool-picker");

    // Then the result pops the picker's own scope, closing the menu.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(tool_picker_scope()))
    );
}

#[rstest::rstest]
#[tokio::test]
async fn escape_after_a_confirm_restores_nothing() {
    // Given an open picker that was confirmed, committing a toggle.
    let wired = Wired::new(three_tools()).await;
    wired.open();
    wired.fire("toggle-highlighted-tool");
    wired.fire("confirm-tool-picker");

    // When escape is pressed again.
    wired.fire("cancel-tool-picker");

    // Then the committed set stands — confirm was authoritative.
    assert!(wired.session_disabled().contains("Bash"));
}

// ── Tab: toggle and advance ─────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn tab_toggles_the_highlighted_tool_off() {
    // Given an open picker whose first row is highlighted and enabled.
    let wired = Wired::new(three_tools()).await;
    wired.open();
    assert_eq!(wired.enabled("Bash"), Some(true));

    // When tab is pressed.
    wired.fire("toggle-highlighted-tool");

    // Then that row is now disabled.
    assert_eq!(wired.enabled("Bash"), Some(false));
}

#[rstest::rstest]
#[tokio::test]
async fn tab_also_advances_the_highlight_to_the_next_row() {
    // Given an open picker whose highlight is on the first row.
    let wired = Wired::new(three_tools()).await;
    wired.open();

    // When tab is pressed.
    wired.fire("toggle-highlighted-tool");

    // Then the highlight moved to the next row, so a run of tools can be
    // toggled in one pass.
    assert_eq!(wired.highlighted(), 1);
}

#[rstest::rstest]
#[tokio::test]
async fn tab_toggles_the_second_tool_once_the_highlight_advanced() {
    // Given an open picker whose highlight was advanced to the second row.
    let wired = Wired::new(three_tools()).await;
    wired.open();
    wired.fire("toggle-highlighted-tool");

    // When tab is pressed again.
    wired.fire("toggle-highlighted-tool");

    // Then the second row is the one that turned off, and the highlight
    // advanced to the third.
    assert_eq!(wired.enabled("edit"), Some(false));
    assert_eq!(wired.highlighted(), 2);
}

#[rstest::rstest]
#[tokio::test]
async fn tab_on_a_picker_with_no_tools_is_a_no_op() {
    // Given a session with no tools registered.
    let wired = Wired::new(vec![]).await;
    wired.open();

    // When tab is pressed.
    wired.fire("toggle-highlighted-tool");

    // Then nothing is selected and the picker is unharmed.
    let state = wired.cell();
    assert!(state.read().selection.selected_item().is_none());
}

// ── Rendering ───────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn each_row_draws_its_toggle_marker_name_and_description() {
    // Given an open picker holding one enabled tool.
    let wired = Wired::new(vec![Def::plain("bash", "Run shell")]).await;
    wired.open();

    // When the popup is drawn.
    let frame = wired.draw();

    // Then the marker, the name, and the description all appear.
    let rendered: String = frame.concat();
    assert!(
        rendered.contains('\u{2713}')
            && rendered.contains("bash")
            && rendered.contains("Run shell"),
        "a tool row draws its marker, name, and description; got {rendered:?}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn the_status_line_reports_the_live_enabled_count() {
    // Given an open picker with one of three tools disabled.
    let wired = Wired::new(three_tools()).await;
    wired
        .state
        .borrow_mut()
        .active_session_mut()
        .set_disabled_tools(["read"].iter().map(|s| (*s).to_owned()).collect());
    wired.open();

    // When the popup is drawn.
    let frame = wired.draw();

    // Then the status line reads 2/3 enabled.
    let rendered: String = frame.concat();
    assert!(
        rendered.contains("2/3 enabled"),
        "the status line reports the enabled count; got {rendered:?}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn the_status_line_tracks_a_toggle_live() {
    // Given an open picker with all three tools enabled.
    let wired = Wired::new(three_tools()).await;
    wired.open();
    assert!(wired.draw().concat().contains("3/3 enabled"));

    // When one row is toggled off.
    wired.fire("toggle-highlighted-tool");

    // Then the status line reflects the new count on the next frame.
    let rendered: String = wired.draw().concat();
    assert!(
        rendered.contains("2/3 enabled"),
        "the status line must follow the toggle; got {rendered:?}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn the_published_viewport_matches_the_rows_the_drawn_popup_shows() {
    // Given an open picker with more rows than one frame can show.
    let many: Vec<Def> = (0..200)
        .map(|i| Def::plain_owned(format!("t{i:03}"), "A tool".to_owned()))
        .collect();
    let wired = Wired::new(many).await;
    wired.open();

    // When a frame is drawn and the rows the buffer actually shows are counted.
    let frame = ratatui::layout::Rect::new(0, 0, 100, 30);
    let popup = crate::tool_picker_render::tool_picker_overlay_rect(&frame)
        .expect("geometry fn yields a popup rect");
    let rows = draw_frame(&wired, frame);
    let drawn_result_rows = count_result_rows(&rows, popup);

    // Then the published viewport equals the drawn result rows, so paging by it
    // keeps the highlight on screen instead of scrolling it out of view.
    let published = wired.cell().read().results_viewport;
    assert_eq!(published, drawn_result_rows);
}

/// Counts the populated result rows inside a drawn popup, excluding the border,
/// filter, separator, and footer chrome.
fn count_result_rows(rows: &[String], popup: ratatui::layout::Rect) -> usize {
    // The result rows sit below the filter and separator, above the footers.
    let first_row = usize::from(popup.y) + 3;
    let last_row = usize::from(popup.y + popup.height) - 3;
    let inner_width = usize::from(popup.width) - 2;
    rows.iter()
        .enumerate()
        .skip(first_row)
        .take_while(|(i, _)| *i < last_row)
        .filter(|(_, line)| {
            line.chars()
                .skip(1) // the popup's left border
                .take(inner_width)
                .any(|c| !c.is_whitespace())
        })
        .count()
}

#[rstest::rstest]
#[tokio::test]
async fn drawing_a_frame_publishes_the_measured_row_count_for_paging() {
    // Given an open picker whose cell starts on the pre-render fallback.
    let wired = Wired::new(three_tools()).await;
    wired.open();
    let fallback = wired.cell().read().results_viewport;
    assert_eq!(
        fallback,
        jinn_tools_msg::TOOL_PICKER_RESULTS_VIEWPORT_FALLBACK
    );

    // When a short frame is drawn — one that lays out fewer rows than the
    // pre-render fallback assumed.
    let frame = ratatui::layout::Rect::new(0, 0, 100, 16);
    draw_frame(&wired, frame);

    // Then the cell carries what the frame actually laid out, so paging and
    // the tab advance move by a page of what is on screen rather than a fixed
    // guess.
    let measured = wired.cell().read().results_viewport;
    let popup = crate::tool_picker_render::tool_picker_overlay_rect(&frame)
        .expect("geometry fn yields a popup rect");
    let expected = crate::tool_picker_viewport::results_viewport(popup);
    assert_eq!(measured, expected);
    assert_ne!(
        measured, fallback,
        "a narrow popup lays out far fewer rows than the fallback, so the \
         measurement must replace it rather than sit beside it"
    );
}

// ── 6. Every advertised key is bound ────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn every_key_the_footer_advertises_is_bound_in_the_picker_scope() {
    // Given a wired slice.
    let wired = Wired::new(three_tools()).await;

    // When the set of keys the picker attached is read.
    let attached: Vec<&'static str> = wired
        .routes
        .rows()
        .iter()
        .filter(|row| row.scope == tool_picker_scope())
        .map(|row| row.key)
        .collect();

    // Then every key the footer advertises is among them — no dead key.
    for bind in crate::tool_picker_render::tool_picker_binds() {
        assert!(
            attached.contains(&bind.notation),
            "the footer advertises {} but the scope binds {attached:?}",
            bind.notation
        );
    }
}

#[rstest::rstest]
#[tokio::test]
async fn the_footer_advertises_the_toggle_apply_and_cancel_keys() {
    // Given a wired slice.
    let wired = Wired::new(three_tools()).await;

    // When the picker's footer rows are read.
    let advertised: Vec<&'static str> = crate::tool_picker_render::tool_picker_binds()
        .iter()
        .map(|bind| bind.notation)
        .collect();

    // Then they are the picker's own toggle / apply / cancel triple, each
    // reachable through an action the slice implements.
    assert_eq!(advertised, vec!["<tab>", "<enter>", "<esc>"]);
    for key in &advertised {
        assert!(
            bound_through_an_action(&wired, key),
            "the footer advertises {key}, so the picker must bind it through an action"
        );
    }
}

/// Whether the picker's scope binds `key` through an action row.
fn bound_through_an_action(wired: &Wired, key: &str) -> bool {
    wired.routes.rows().iter().any(|row| {
        row.scope == tool_picker_scope()
            && row.key == key
            && matches!(row.outcome, RouteOutcome::Action { .. })
    })
}

// ── 7. One owner ────────────────────────────────────────────────────────

#[rstest::rstest]
fn the_picker_state_lives_only_in_its_slice_cell() {
    // The tool picker's state is reachable from exactly one place: the slice
    // cell. A second copy in the kernel would let the menu show one store while
    // a different one is written.
    let kernel_source = include_str!("../../../jinn-domain/src/state/frontend_state.rs");
    assert!(
        !kernel_source.contains("tool_picker"),
        "the kernel must not hold tool picker state; the slice cell is the only home"
    );
}

#[rstest::rstest]
fn the_kernel_names_no_tool_picker_at_all() {
    // The central app crate and the TUI layer must not know this picker exists:
    // no scope variant, no picker kind, no spec id. That is what makes adding a
    // picker a folder-local change.
    for (label, source) in [
        (
            "jinn-domain frontend state",
            include_str!("../../../jinn-domain/src/state/frontend_state.rs"),
        ),
        (
            "jinn-domain intent handler",
            include_str!("../../../jinn-domain/src/feat/intent/handler.rs"),
        ),
        (
            "jinn-domain protocol intents",
            include_str!("../../../jinn-domain/src/protocol/intent.rs"),
        ),
        (
            "jinn-tui scope table",
            include_str!("../../../jinn-tui/src/scope.rs"),
        ),
    ] {
        let picker_named = [
            "PickerTool",
            "Picker(tool)",
            "TOOL_ID",
            "tool_spec",
            "PickerKind::Tool",
        ];
        let hits: Vec<&str> = picker_named
            .iter()
            .filter(|needle| source.contains(*needle))
            .copied()
            .collect();
        assert!(
            hits.is_empty(),
            "{label} still names the tool picker ({hits:?}); the slice must own it entirely"
        );
    }
}

/// Owned-name helper so the many-tool fixtures can build definitions at
/// runtime while the static ones stay literals.
impl Def {
    /// A plain tool with runtime-built names, for the paging fixtures.
    fn plain_owned(name: String, description: String) -> Self {
        Self {
            name: Box::leak(name.into_boxed_str()),
            description: Box::leak(description.into_boxed_str()),
            provider_gated: false,
        }
    }
}
