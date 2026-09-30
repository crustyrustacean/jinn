//! Tests for the attendant properties popup: the form's rows and the seed
//! template editor's rows, exercised through route dispatch against a real
//! `AppState`.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use jinn_app_state::AppState;
use jinn_attendant_msg::{
    AttendantBehavior, AttendantPropertiesState, AttendantTrigger, OriginalValues, PickDirection,
    PopupStatus, PropertyField, SetField, SetMode, attendant_properties_scope,
    attendant_properties_slot, attendant_seed_template_scope,
};
use jinn_session_state::ChatSessionState;
use jinn_slices::KeyRoutes;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{ActionCtx, DynamicIntent, ScopeSignal};

use crate::properties_overlay::{
    attach_properties_rows, attach_seed_template_rows, register_seed_template_input_hook,
};

/// The popup's cell over a fresh `Slices` registry, registered the same way
/// the cell catalog does.
fn popup_cell() -> TypedCell<AttendantPropertiesState> {
    let slices = jinn_slices::Slices::new();
    slices
        .register(
            attendant_properties_slot(),
            AttendantPropertiesState::default(),
        )
        .expect("the slot is unclaimed on a fresh registry")
}

/// An [`ActionCtx`] over a real [`AppState`], as the intent handler lends it.
fn action_ctx<'a>(state: &'a mut AppState, slices: &'a jinn_slices::Slices) -> ActionCtx<'a> {
    ActionCtx {
        state,
        slices,
        config: jinn_kernel::common::render_ctx::empty_config_layer(),
        key_bytes: Vec::new(),
    }
}

/// Runs a properties-scope row's action through the route table.
fn run_properties_row(
    routes: &KeyRoutes,
    ctx: ActionCtx<'_>,
    cell: &TypedCell<AttendantPropertiesState>,
    action: &'static str,
) -> jinn_slices::RouteResult {
    let _ = cell;
    let scope = attendant_properties_scope();
    let intent = DynamicIntent::new(scope, action, "test");
    routes
        .action_for(&intent, ctx)
        .unwrap_or_else(|| panic!("no row bound for action {action:?}"))
}

/// Runs an editor-scope row's action through the route table.
fn run_editor_row(
    routes: &KeyRoutes,
    ctx: ActionCtx<'_>,
    action: &'static str,
) -> jinn_slices::RouteResult {
    let scope = attendant_seed_template_scope();
    let intent = DynamicIntent::new(scope, action, "test");
    routes
        .action_for(&intent, ctx)
        .unwrap_or_else(|| panic!("no row bound for action {action:?}"))
}

/// Dispatches the intent's scope signal against the state's scope stack,
/// the way the kernel handler does.
fn apply_scope_signal(state: &mut AppState, signal: Option<ScopeSignal>) {
    let Some(signal) = signal else { return };
    match signal {
        ScopeSignal::Push(scope) => state
            .frontend
            .scope_push(jinn_slices::FocusScope::Dynamic(scope)),
        ScopeSignal::PopIf(scope) => {
            if state.frontend.scope() == jinn_slices::FocusScope::Dynamic(scope) {
                state.frontend.scope_pop();
            }
        }
    }
}

/// An AppState whose active session is an attendant, with a cell + routes
/// wired the way activation does, and the popup cell seeded from the session.
struct PopupFixture {
    state: AppState,
    slices: jinn_slices::Slices,
    routes: KeyRoutes,
    cell: TypedCell<AttendantPropertiesState>,
    attendant_id: jinn_core_types::SessionId,
}

impl PopupFixture {
    /// Builds the fixture with the attendant's values seeded into the popup
    /// cell, as the sidebar opener does.
    fn new() -> Self {
        let mut state = AppState::default_with_scope_focus();
        let attendant_id = {
            let parent = ChatSessionState::new();
            let attendant = ChatSessionState::new_attendant(&parent, true);
            let id = attendant.session_id().clone();
            state.session.insert(attendant);
            id
        };
        let slices = jinn_slices::Slices::new();
        let cell = slices
            .register(
                attendant_properties_slot(),
                AttendantPropertiesState::default(),
            )
            .expect("unclaimed slot");
        let routes = KeyRoutes::new();
        attach_properties_rows(&routes, &cell);
        attach_seed_template_rows(&routes, &cell);
        register_seed_template_input_hook(&routes, &cell);
        Self {
            state,
            slices,
            routes,
            cell,
            attendant_id,
        }
    }

    /// Opens the popup over the attendant, mirroring the opener's seeding.
    fn open(&mut self) {
        let session = self
            .state
            .session
            .get(&self.attendant_id)
            .expect("attendant");
        let template = session.seed_template().to_owned();
        let cursor_pos = template.len();
        let prep_mode = session.attendant_is_prepping();
        let popup = AttendantPropertiesState {
            session_id: Some(self.attendant_id.clone()),
            seed_template: jinn_slices::LineInput {
                input: template.clone(),
                cursor_pos,
            },
            pending_behavior: session.attendant_behavior(),
            pending_trigger: session.attendant_trigger(),
            pending_prep_mode: prep_mode,
            pending_tool_set: OriginalValues::mode_of(session.tool_filter()),
            frozen_tools: OriginalValues::names_of(session.tool_filter()),
            pending_skill_set: OriginalValues::mode_of(session.skill_filter()),
            frozen_skills: OriginalValues::names_of(session.skill_filter()),
            original: Some(OriginalValues {
                trigger: session.attendant_trigger(),
                behavior: session.attendant_behavior(),
                prep_mode,
                tool_set: session.tool_filter().clone(),
                skill_set: session.skill_filter().clone(),
                template,
            }),
            // The opener's rule: a composing attendant opens on the prep
            // row, anything else on the first field.
            focus: PropertyField::opening_focus(prep_mode),
            ..AttendantPropertiesState::default()
        };
        self.cell.update(|s| *s = popup);
        self.state
            .frontend
            .scope_push(jinn_slices::FocusScope::Dynamic(
                attendant_properties_scope(),
            ));
    }

    /// Ends composition on the underlying session, as leaving prep mode
    /// in the panel would.
    fn finish_composing(&mut self) {
        self.state
            .session
            .get_mut(&self.attendant_id)
            .expect("attendant")
            .set_attendant_is_prepping(false);
    }

    /// Opens the popup and walks the cursor down to the seed-template field.
    ///
    /// A fresh attendant is composing, and the cage floors the walk at the
    /// prep row. These tests are about the rows below it, so the fixture
    /// ends composition before the walk — the walk is then one press per
    /// field above the template, and the template is last of six.
    fn open_on_the_template_field(&mut self) {
        self.finish_composing();
        self.open();
        for _ in 0..5 {
            self.press("attendant-properties-field-next");
        }
        assert_eq!(self.cell.read().focus, PropertyField::SeedTemplate);
    }

    /// Reads the attendant session's current values.
    fn session_values(&self) -> (AttendantTrigger, AttendantBehavior, String) {
        let session = self
            .state
            .session
            .get(&self.attendant_id)
            .expect("attendant");
        (
            session.attendant_trigger(),
            session.attendant_behavior(),
            session.seed_template().to_owned(),
        )
    }

    /// Runs a properties row and applies any scope signal to the state.
    fn press(&mut self, action: &'static str) -> jinn_slices::RouteResult {
        let ctx = action_ctx(&mut self.state, &self.slices);
        let result = run_properties_row(&self.routes, ctx, &self.cell, action);
        apply_scope_signal(&mut self.state, result.scope_signal.clone());
        result
    }

    /// Runs an editor row and applies any scope signal to the state.
    fn press_editor(&mut self, action: &'static str) -> jinn_slices::RouteResult {
        let ctx = action_ctx(&mut self.state, &self.slices);
        let result = run_editor_row(&self.routes, ctx, action);
        apply_scope_signal(&mut self.state, result.scope_signal.clone());
        result
    }
}

#[rstest::rstest]
#[test]
fn j_and_k_stop_at_the_ends_of_the_form() {
    // Given an open popup over a composed attendant, focused on the first
    // field.
    let mut fx = PopupFixture::new();
    fx.finish_composing();
    fx.open();
    assert_eq!(fx.cell.read().focus, PropertyField::Trigger);

    // When pressing k at the top of the form.
    fx.press("attendant-properties-field-previous");

    // Then the cursor stays on the first field rather than wrapping to the
    // bottom. Silently teleporting the cursor is how a user loses track of
    // which row they are about to change.
    assert_eq!(fx.cell.read().focus, PropertyField::Trigger);

    // And when pressing j repeatedly past the bottom, it stops there too.
    for _ in 0..5 {
        fx.press("attendant-properties-field-next");
    }
    assert_eq!(fx.cell.read().focus, PropertyField::SeedTemplate);
    // And the rows above it are one press each, in reverse display order.
    fx.press("attendant-properties-field-previous");
    assert_eq!(fx.cell.read().focus, PropertyField::SkillSet);
    fx.press("attendant-properties-field-previous");
    assert_eq!(fx.cell.read().focus, PropertyField::ToolSet);
    fx.press("attendant-properties-field-previous");
    assert_eq!(fx.cell.read().focus, PropertyField::PrepMode);
    fx.press("attendant-properties-field-previous");
    assert_eq!(fx.cell.read().focus, PropertyField::Behavior);
}

#[rstest::rstest]
#[test]
fn a_composing_attendant_opens_with_the_cursor_on_the_prep_row() {
    // Given a fresh attendant, which `N` leaves composing.
    let mut fx = PopupFixture::new();

    // When the popup opens.
    fx.open();

    // Then the cursor rests on the prep row, not the trigger. The two rows
    // above do not apply while composing, and a cursor on one of them would
    // be sitting on a control that cannot be operated.
    assert_eq!(fx.cell.read().focus, PropertyField::PrepMode);
}

#[rstest::rstest]
#[test]
fn the_cursor_cannot_climb_above_the_prep_row_while_composing() {
    // Given an open popup over a composing attendant.
    let mut fx = PopupFixture::new();
    fx.open();

    // When pressing k twice from the prep row — enough to cross both rows
    // that do not apply.
    fx.press("attendant-properties-field-previous");
    fx.press("attendant-properties-field-previous");

    // Then the cursor is still on the prep row. The rows are on screen and
    // dimmed, but the cursor cannot land on them.
    assert_eq!(fx.cell.read().focus, PropertyField::PrepMode);
}

#[rstest::rstest]
#[test]
fn leaving_prep_mode_reaches_the_rows_above_it() {
    // Given an open popup over a composing attendant, cursor on the prep
    // row.
    let mut fx = PopupFixture::new();
    fx.open();
    assert_eq!(fx.cell.read().focus, PropertyField::PrepMode);

    // When turning prep mode off and pressing k.
    fx.press("attendant-properties-pick-left");
    fx.press("attendant-properties-field-previous");

    // Then the cursor lands on the behavior row: the row directly above the
    // prep row became live the moment composition ended.
    assert!(!fx.cell.read().pending_prep_mode);
    assert_eq!(fx.cell.read().focus, PropertyField::Behavior);
}

#[rstest::rstest]
#[test]
fn either_pick_key_turns_composition_off_from_the_prep_row() {
    // Given an open popup over a composing attendant, cursor on the prep
    // row.
    let mut fx = PopupFixture::new();
    fx.open();
    assert!(fx.cell.read().pending_prep_mode);

    // When pressing l — the other pick key.
    fx.press("attendant-properties-pick-right");

    // Then composition ended. A two-state field has no previous or next, so
    // both pick keys cycle it the same way.
    assert!(!fx.cell.read().pending_prep_mode);
}

#[rstest::rstest]
#[test]
fn h_and_l_pick_choices_in_place() {
    // Given an open popup focused on the trigger row, whose pending value
    // is the rightmost choice (a fresh attendant's `manual`).
    let mut fx = PopupFixture::new();
    fx.finish_composing();
    fx.open();
    assert_eq!(fx.cell.read().focus, PropertyField::Trigger);
    assert_eq!(fx.cell.read().pending_trigger, AttendantTrigger::Manual);

    // When picking left.
    fx.press("attendant-properties-pick-left");

    // Then the pending trigger moved one choice left.
    assert_eq!(
        fx.cell.read().pending_trigger,
        AttendantTrigger::ParentCompleted
    );

    // When picking right again.
    fx.press("attendant-properties-pick-right");

    // Then it moved back to manual.
    assert_eq!(fx.cell.read().pending_trigger, AttendantTrigger::Manual);
}

#[rstest::rstest]
#[test]
fn picking_clamps_at_row_ends() {
    // Given the behavior row focused on its leftmost choice.
    let mut fx = PopupFixture::new();
    fx.finish_composing();
    fx.open();
    fx.press("attendant-properties-field-previous"); // template -> prep mode
    assert_eq!(fx.cell.read().pending_behavior, AttendantBehavior::Reset);

    // When picking left at the left end.
    fx.press("attendant-properties-pick-left");

    // Then the choice does not move.
    assert_eq!(fx.cell.read().pending_behavior, AttendantBehavior::Reset);
}

#[rstest::rstest]
#[test]
fn picking_does_not_touch_the_session_until_enter() {
    // Given an open popup with a pick applied on the trigger row.
    let mut fx = PopupFixture::new();
    fx.open();
    fx.press("attendant-properties-field-next");
    fx.press("attendant-properties-field-next");
    fx.press("attendant-properties-pick-right");

    // When checking the session before applying.
    let (trigger, _behavior, _template) = fx.session_values();

    // Then the session still holds its original trigger: picks are pending.
    assert_eq!(trigger, AttendantTrigger::Manual);
}

#[rstest::rstest]
#[test]
fn enter_applies_all_fields_and_persists_once() {
    // Given an open popup with every field edited. The fresh attendant's
    // trigger starts at `manual` (rightmost), so the pick goes left; the
    // behavior starts at `reset` (leftmost), so it goes right; and prep
    // mode starts on, so it is turned off — which is what makes the other
    // two edits applicable in the first place.
    let mut fx = PopupFixture::new();
    fx.open();
    assert_eq!(fx.cell.read().focus, PropertyField::PrepMode);
    fx.press("attendant-properties-pick-left"); // prep mode -> off
    fx.press("attendant-properties-field-previous"); // prep mode -> behavior
    fx.cell.update(|p| p.pick(PickDirection::Right)); // reset -> preserve
    fx.press("attendant-properties-field-previous"); // behavior -> trigger
    fx.press("attendant-properties-pick-left"); // trigger -> parent-completed
    fx.cell
        .update(|p| p.seed_template.input = "edited template".to_owned());

    // When applying.
    let result = fx.press("attendant-properties-apply");

    // Then the session holds every edited value together, composition
    // included.
    assert_eq!(
        fx.session_values(),
        (
            AttendantTrigger::ParentCompleted,
            AttendantBehavior::Preserve,
            "edited template".to_owned()
        )
    );
    assert!(
        !fx.state
            .session
            .get(&fx.attendant_id)
            .expect("attendant")
            .attendant_is_prepping(),
        "applying must end composition, or nothing would ever run"
    );
    // And exactly one persist was published.
    assert_eq!(
        result.message_names,
        vec!["PersistSession"],
        "expected exactly one PersistSession, got {:?}",
        result.message_names
    );
}

#[rstest::rstest]
#[test]
fn enter_marks_a_fresh_attendant_interacted() {
    // Given a fresh attendant (never interacted) with an open popup.
    let mut fx = PopupFixture::new();
    fx.open();
    assert!(
        !fx.state
            .session
            .get(&fx.attendant_id)
            .expect("attendant")
            .has_interacted()
    );

    // When applying.
    fx.press("attendant-properties-apply");

    // Then the session is interacted, so the persist is not dropped.
    assert!(
        fx.state
            .session
            .get(&fx.attendant_id)
            .expect("attendant")
            .has_interacted()
    );
}

#[rstest::rstest]
#[test]
fn enter_pops_the_properties_scope() {
    // Given an open popup.
    let mut fx = PopupFixture::new();
    fx.open();

    // When applying.
    fx.press("attendant-properties-apply");

    // Then the properties scope popped.
    assert_ne!(
        fx.state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(attendant_properties_scope())
    );
}

#[rstest::rstest]
#[test]
fn escape_restores_original_values_and_pops() {
    // Given an open popup over an attendant with a known template, with
    // all three fields edited since.
    let mut fx = PopupFixture::new();
    fx.state
        .session
        .get_mut(&fx.attendant_id)
        .expect("attendant")
        .set_seed_template("original".to_owned());
    fx.open();
    fx.press("attendant-properties-field-next");
    fx.press("attendant-properties-pick-left");
    fx.cell.update(|p| {
        p.seed_template.input = "scrap".to_owned();
    });

    // When cancelling with escape.
    let result = fx.press("attendant-properties-leave");

    // Then nothing persists.
    assert!(result.message_names.is_empty());
    // And the session keeps its open-time values.
    assert_eq!(
        fx.session_values(),
        (
            AttendantTrigger::Manual,
            AttendantBehavior::Reset,
            "original".to_owned()
        )
    );
    // And the popup cell was restored too.
    let popup = fx.cell.read().clone();
    assert_eq!(popup.pending_trigger, AttendantTrigger::Manual);
    assert_eq!(popup.seed_template.input, "original");
    // And the scope popped.
    assert_ne!(
        fx.state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(attendant_properties_scope())
    );
}

#[rstest::rstest]
#[test]
fn ctrl_c_cancels_like_escape_on_properties() {
    // Given an open popup with an edit pending.
    let mut fx = PopupFixture::new();
    fx.open();
    fx.press("attendant-properties-field-next");
    fx.press("attendant-properties-pick-right");

    // When cancelling with ctrl-c.
    fx.press("attendant-properties-cancel");

    // Then the pending edit is discarded.
    assert_eq!(fx.cell.read().pending_trigger, AttendantTrigger::Manual);
    // And the scope popped.
    assert_ne!(
        fx.state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(attendant_properties_scope())
    );
}

#[rstest::rstest]
#[test]
fn i_on_template_field_pushes_the_editor_scope() {
    // Given an open popup focused on the seed template.
    let mut fx = PopupFixture::new();
    fx.open_on_the_template_field();

    // When pressing i.
    fx.press("attendant-properties-edit-template");

    // Then the editor scope pushed on top of the properties scope.
    assert_eq!(
        fx.state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(attendant_seed_template_scope())
    );
    // And the pre-editor text was captured (a fresh attendant seeds the
    // default template).
    let popup = fx.cell.read().clone();
    assert_eq!(
        popup.editor_original,
        Some(popup.original.expect("opened").template)
    );
}

#[rstest::rstest]
#[test]
fn i_off_the_template_field_is_a_noop() {
    // Given an open popup over a composed attendant, focused on the
    // behavior row — a choice row, not the template row.
    let mut fx = PopupFixture::new();
    fx.finish_composing();
    fx.open();
    fx.press("attendant-properties-field-next");
    assert_eq!(fx.cell.read().focus, PropertyField::Behavior);

    // When pressing i.
    fx.press("attendant-properties-edit-template");

    // Then no scope pushed.
    assert_eq!(
        fx.state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(attendant_properties_scope())
    );
    // And the editor was not opened.
    assert!(fx.cell.read().editor_original.is_none());
}

#[rstest::rstest]
#[test]
fn editor_enter_keeps_text_and_pops() {
    // Given an open editor with edited text.
    let mut fx = PopupFixture::new();
    fx.open();
    fx.press("attendant-properties-edit-template");
    fx.cell
        .update(|p| p.seed_template.input = "kept".to_owned());

    // When pressing enter in the editor.
    fx.press_editor("attendant-template-keep");

    // Then the text stands in the draft.
    assert_eq!(fx.cell.read().seed_template.input, "kept");
    // And the editor closed.
    assert_eq!(
        fx.state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(attendant_properties_scope())
    );
    // And the draft stays pending: the session is untouched until apply.
    let (_t, _a, template) = fx.session_values();
    assert_ne!(template, "kept");
}

#[rstest::rstest]
#[test]
fn editor_escape_restores_pre_editor_text() {
    // Given an open editor whose draft changed.
    let mut fx = PopupFixture::new();
    fx.open_on_the_template_field();
    fx.cell
        .update(|p| p.seed_template.input = "before".to_owned());
    fx.press("attendant-properties-edit-template");
    fx.cell
        .update(|p| p.seed_template.input = "during".to_owned());

    // When pressing escape in the editor.
    fx.press_editor("attendant-template-restore");

    // Then the draft is back to the pre-editor text.
    assert_eq!(fx.cell.read().seed_template.input, "before");
    // And the editor closed back onto the properties scope.
    assert_eq!(
        fx.state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(attendant_properties_scope())
    );
}

#[rstest::rstest]
#[test]
fn editor_ctrl_c_clears_then_leaves_when_empty() {
    // Given an open editor with text.
    let mut fx = PopupFixture::new();
    fx.open_on_the_template_field();
    fx.cell
        .update(|p| p.seed_template.input = "text".to_owned());
    fx.press("attendant-properties-edit-template");

    // When pressing ctrl-c.
    let result = fx.press_editor("attendant-template-clear-or-leave");

    // Then the text cleared and the editor stays open.
    assert_eq!(fx.cell.read().seed_template.input, "");
    assert!(result.scope_signal.is_none());
    assert_eq!(
        fx.state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(attendant_seed_template_scope())
    );

    // When pressing ctrl-c again on the empty draft.
    let result = fx.press_editor("attendant-template-clear-or-leave");

    // Then the editor pops.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(attendant_seed_template_scope()))
    );
}

#[rstest::rstest]
#[test]
fn editor_typing_reaches_the_draft_through_the_hook() {
    // Given the editor open with its hook registered on the editor scope.
    let mut fx = PopupFixture::new();
    fx.open();
    fx.press("attendant-properties-edit-template");

    // When the hook receives an insert-char edit intent.
    let hook = fx
        .routes
        .input_hook(&attendant_seed_template_scope())
        .expect("editor hook registered");
    let intent = jinn_slices::route::EditIntent::InsertChar('x');
    let _ = hook(&intent);

    // Then the draft holds the character.
    assert!(fx.cell.read().seed_template.input.contains('x'));
}

#[rstest::rstest]
#[test]
fn editor_typing_never_reaches_the_chat_input() {
    // Given the editor open over a state whose chat input holds its own
    // text (as it would behind the popup).
    let mut fx = PopupFixture::new();
    fx.open();
    fx.state
        .update_active_input(|input| input.insert_text("behind"));
    fx.press("attendant-properties-edit-template");

    // When an insert-char editing key fires while the editor scope is on
    // top, the kernel resolves it through the editor scope's hook (the
    // keymap mints `insert-char` dynamic intents for capturing dynamic
    // scopes; the chat input's catch-all lives on `Scope::Input` only).
    let hook = fx
        .routes
        .input_hook(&attendant_seed_template_scope())
        .expect("editor hook registered");
    let _ = hook(&jinn_slices::route::EditIntent::InsertChar('x'));

    // Then the character landed in the template draft…
    assert!(fx.cell.read().seed_template.input.contains('x'));
    // …and the chat input behind the popup is untouched.
    let chat_input = fx
        .state
        .active_session()
        .with_input(|i| i.text().to_owned(), String::new);
    assert_eq!(chat_input, "behind");
}

#[rstest::rstest]
#[test]
fn properties_scope_has_no_input_hook() {
    // Given the popup rows attached as at activation.

    // When resolving the properties scope's input hook.
    let routes = {
        let cell = popup_cell();
        let routes = KeyRoutes::new();
        attach_properties_rows(&routes, &cell);
        attach_seed_template_rows(&routes, &cell);
        register_seed_template_input_hook(&routes, &cell);
        routes
    };

    // Then there is none: the navigation-only form captures no input.
    assert!(routes.input_hook(&attendant_properties_scope()).is_none());
}

// ── Tool and skill sets ─────────────────────────────────────────────────

/// A discovered skill named `name`.
fn skill(name: String) -> jinn_skills_msg::Skill {
    jinn_skills_msg::Skill {
        name,
        description: "a skill".to_owned(),
        body: String::new(),
        file_path: std::path::PathBuf::from("/tmp/skill/SKILL.md"),
        base_dir: std::path::PathBuf::from("/tmp/skill"),
        source: jinn_skills_msg::SkillSource::Global,
    }
}

/// A plain function tool named `name` — available to every provider.
fn tool(name: &str) -> jinn_core_types::ToolDefinition {
    jinn_core_types::ToolDefinition {
        name: name.to_owned(),
        description: format!("{name} description"),
        parameters: jinn_testutil::json_value(),
        prompt_snippet: None,
        prompt_guidelines: Vec::new(),
        server_tool_type: None,
    }
}

impl PopupFixture {
    /// Puts `tools` and `skills` in the attendant's registry and discovered
    /// set, as context assembly would.
    fn offer(&mut self, tools: &[&str]) {
        self.state
            .tool_registry()
            .expect("the cell catalog registers the tool registry")
            .update(|registry| {
                for name in tools {
                    registry.global.insert((*name).to_owned(), tool(name));
                }
            });
    }

    /// Puts `skills` in the attendant's discovered set, as discovery would.
    fn offer_skills(&mut self, skills: &[&str]) {
        self.state
            .session
            .get_mut(&self.attendant_id)
            .expect("attendant")
            .set_discovered_skills(
                skills
                    .iter()
                    .map(|name| skill((*name).to_owned()))
                    .collect(),
            );
    }

    /// Gives the attendant the given tool filter.
    fn with_tool_filter(&mut self, filter: jinn_core_types::NameFilter) {
        self.state
            .session
            .get_mut(&self.attendant_id)
            .expect("attendant")
            .set_tool_filter(filter);
    }

    /// Focuses the tool-set row.
    fn open_on_the_tool_set(&mut self) {
        self.finish_composing();
        self.open();
        fx_press_n(self, 3);
        assert_eq!(self.cell.read().focus, PropertyField::ToolSet);
    }

    /// Focuses the skill-set row.
    fn open_on_the_skill_set(&mut self) {
        self.finish_composing();
        self.open();
        fx_press_n(self, 4);
        assert_eq!(self.cell.read().focus, PropertyField::SkillSet);
    }

    /// The attendant's current tool filter.
    fn tool_filter(&self) -> jinn_core_types::NameFilter {
        self.state
            .session
            .get(&self.attendant_id)
            .expect("attendant")
            .tool_filter()
            .clone()
    }

    /// The attendant's current skill filter.
    fn skill_filter(&self) -> jinn_core_types::NameFilter {
        self.state
            .session
            .get(&self.attendant_id)
            .expect("attendant")
            .skill_filter()
            .clone()
    }
}

/// Presses the next-field row `count` times on `fx`.
fn fx_press_n(fx: &mut PopupFixture, count: usize) {
    for _ in 0..count {
        fx.press("attendant-properties-field-next");
    }
}

/// An allow filter over `names`.
fn allow(names: &[&str]) -> jinn_core_types::NameFilter {
    jinn_core_types::NameFilter {
        mode: jinn_core_types::FilterMode::Allow,
        names: names.iter().map(|n| (*n).to_owned()).collect(),
    }
}

#[rstest::rstest]
#[test]
fn a_fresh_attendant_opens_with_both_sets_live() {
    // Given an attendant whose filters are unconfigured.
    let mut fx = PopupFixture::new();

    // When the popup opens.
    fx.open();

    // Then both set rows read Live: nothing about this attendant pins a
    // capability set, so it follows the user's configuration as it grows.
    assert_eq!(fx.cell.read().set_mode_of(SetField::Tool), SetMode::Live);
    assert_eq!(fx.cell.read().set_mode_of(SetField::Skill), SetMode::Live);
}

#[rstest::rstest]
#[test]
fn an_allow_filtered_attendant_opens_with_its_tool_set_frozen() {
    // Given an attendant already pinned to two tools.
    let mut fx = PopupFixture::new();
    fx.with_tool_filter(allow(&["read", "write"]));

    // When the popup opens.
    fx.open();

    // Then the tool set row reads Frozen, carrying the filter's own names.
    assert_eq!(fx.cell.read().set_mode_of(SetField::Tool), SetMode::Frozen);
    assert_eq!(
        fx.cell.read().pending_set(SetField::Tool),
        Some(&allow(&["read", "write"]).names)
    );
}

#[rstest::rstest]
#[test]
fn a_blocklisted_attendant_opens_with_its_tool_set_live() {
    // Given an attendant with a blocklist — the shape the pickers write.
    let mut fx = PopupFixture::new();
    fx.with_tool_filter(jinn_core_types::NameFilter::deny(["bash".to_owned()]));

    // When the popup opens.
    fx.open();

    // Then the tool set row reads Live. A blocklist says "never these" and
    // nothing about the rest, so it pins no set.
    assert_eq!(fx.cell.read().set_mode_of(SetField::Tool), SetMode::Live);
}

#[rstest::rstest]
#[test]
fn freezing_the_tool_set_does_not_touch_the_session() {
    // Given an open popup over an attendant with no filter.
    let mut fx = PopupFixture::new();
    fx.offer(&["read", "write"]);
    fx.open_on_the_tool_set();

    // When freezing the row.
    fx.press("attendant-properties-pick-right");

    // Then the row is Frozen…
    assert_eq!(fx.cell.read().set_mode_of(SetField::Tool), SetMode::Frozen);
    // …and the attendant's filter is still exactly what it was. The popup
    // buffers; nothing reaches the session until `<enter>`.
    assert!(fx.tool_filter().is_unconfigured());
}

#[rstest::rstest]
#[test]
fn freezing_the_tool_set_captures_the_names_the_attendant_permits() {
    // Given an attendant that a blocklist withholds one tool from.
    let mut fx = PopupFixture::new();
    fx.offer(&["read", "write", "bash"]);
    fx.with_tool_filter(jinn_core_types::NameFilter::deny(["bash".to_owned()]));
    fx.open_on_the_tool_set();

    // When freezing the row.
    fx.press("attendant-properties-pick-right");

    // Then the capture is the permitted set — the blocklist's whole point
    // is that `bash` is not in it.
    assert_eq!(
        fx.cell.read().pending_set(SetField::Tool),
        Some(&allow(&["read", "write"]).names)
    );
}

#[rstest::rstest]
#[test]
fn freezing_the_skill_set_captures_the_discovered_skills() {
    // Given an attendant with two skills discovered and one withheld.
    let mut fx = PopupFixture::new();
    fx.offer(&[]);
    fx.offer_skills(&["web-coder", "reviewer"]);
    fx.state
        .session
        .get_mut(&fx.attendant_id)
        .expect("attendant")
        .set_skill_filter(jinn_core_types::NameFilter::deny(["reviewer".to_owned()]));
    fx.open_on_the_skill_set();

    // When freezing the row.
    fx.press("attendant-properties-pick-left");

    // Then the capture is the permitted set.
    assert_eq!(
        fx.cell.read().pending_set(SetField::Skill),
        Some(&allow(&["web-coder"]).names)
    );
}

#[rstest::rstest]
#[test]
fn enter_writes_the_frozen_set_as_an_allow_list() {
    // Given a popup with its tool set frozen over two names.
    let mut fx = PopupFixture::new();
    fx.offer(&["read", "write"]);
    fx.open_on_the_tool_set();
    fx.press("attendant-properties-pick-right");

    // When applying.
    fx.press("attendant-properties-apply");

    // Then the attendant's filter is an allow list naming exactly those two,
    // which is what makes a tool registered afterward refused.
    assert_eq!(fx.tool_filter(), allow(&["read", "write"]));
}

#[rstest::rstest]
#[test]
fn a_live_set_writes_nothing_so_the_attendant_inherits() {
    // Given a popup left with both set rows Live.
    let mut fx = PopupFixture::new();
    fx.offer(&["read"]);
    fx.offer_skills(&["web-coder"]);
    fx.open_on_the_tool_set();

    // When applying.
    fx.press("attendant-properties-apply");

    // Then both filters are left unconfigured, so a tool or skill
    // registered later is admitted automatically.
    assert!(fx.tool_filter().is_unconfigured());
    assert!(fx.skill_filter().is_unconfigured());
}

#[rstest::rstest]
#[test]
fn a_frozen_attendant_refuses_a_tool_registered_after_the_freeze() {
    // Given an attendant frozen to the two tools it had.
    let mut fx = PopupFixture::new();
    fx.offer(&["read", "write"]);
    fx.open_on_the_tool_set();
    fx.press("attendant-properties-pick-right");
    fx.press("attendant-properties-apply");

    // When a new tool arrives — an MCP server the user has just added.
    fx.offer(&["read", "write", "mcp__github__create_pr"]);

    // Then the attendant still admits the two it was frozen with and
    // refuses the newcomer.
    let filter = fx.tool_filter();
    assert!(filter.permits("read"));
    assert!(!filter.permits("mcp__github__create_pr"));
}

#[rstest::rstest]
#[test]
fn a_live_attendant_admits_a_tool_registered_after_the_panel_opened() {
    // Given an attendant left with its tool set Live.
    let mut fx = PopupFixture::new();
    fx.offer(&["read"]);
    fx.open_on_the_tool_set();

    // When a new tool arrives.
    fx.offer(&["read", "mcp__github__create_pr"]);

    // Then its filter permits the newcomer: an unconfigured filter places no
    // restriction on names it does not contain.
    assert!(fx.tool_filter().permits("mcp__github__create_pr"));
}

#[rstest::rstest]
#[test]
fn escape_after_a_freeze_leaves_both_filters_untouched() {
    // Given an attendant holding a blocklist and an allow list.
    let mut fx = PopupFixture::new();
    fx.offer(&["read", "write", "bash"]);
    fx.offer_skills(&["web-coder"]);
    let before_tools = jinn_core_types::NameFilter::deny(["bash".to_owned()]);
    let before_skills = allow(&["web-coder"]);
    fx.with_tool_filter(before_tools.clone());
    fx.state
        .session
        .get_mut(&fx.attendant_id)
        .expect("attendant")
        .set_skill_filter(before_skills.clone());
    fx.open_on_the_tool_set();
    // And the tool set is frozen, then thawed, then frozen again.
    fx.press("attendant-properties-pick-right");
    fx.press("attendant-properties-pick-left");
    fx.press("attendant-properties-pick-right");

    // When leaving with escape.
    fx.press("attendant-properties-leave");

    // Then both filters are byte-identical to their open-time values. A
    // popup that had been applied would not close on escape, so reaching
    // this line at all is half the assertion.
    assert_eq!(fx.tool_filter(), before_tools);
    assert_eq!(fx.skill_filter(), before_skills);
}

#[rstest::rstest]
#[test]
fn escape_restores_a_frozen_row_with_the_filters_own_names() {
    // Given an attendant whose tool filter is an allow list written by
    // hand, including a glob.
    let mut fx = PopupFixture::new();
    let hand_written = allow(&["read", "mcp__github__*"]);
    fx.with_tool_filter(hand_written.clone());
    fx.open_on_the_tool_set();

    // When leaving with escape without touching the row.
    fx.press("attendant-properties-leave");

    // Then the row is still Frozen carrying the glob the user wrote, and the
    // attendant's filter is unchanged.
    assert_eq!(fx.cell.read().set_mode_of(SetField::Tool), SetMode::Frozen);
    assert_eq!(
        fx.cell.read().pending_set(SetField::Tool),
        Some(&hand_written.names)
    );
    assert_eq!(fx.tool_filter(), hand_written);
}

#[rstest::rstest]
#[test]
fn thawing_a_frozen_row_clears_its_capture() {
    // Given a popup with its tool set frozen over two names.
    let mut fx = PopupFixture::new();
    fx.offer(&["read", "write"]);
    fx.open_on_the_tool_set();
    fx.press("attendant-properties-pick-right");

    // When thawing it with the other pick key.
    fx.press("attendant-properties-pick-left");

    // Then the row is Live and holds nothing to commit, and no status line
    // is left over from the freeze.
    assert_eq!(fx.cell.read().set_mode_of(SetField::Tool), SetMode::Live);
    assert!(fx.cell.read().pending_set(SetField::Tool).is_none());
    assert!(fx.cell.read().status.is_none());
}

#[rstest::rstest]
#[test]
fn saving_a_thawed_attendant_leaves_its_filter_unconfigured() {
    // Given a popup frozen over two tools and then thawed again.
    let mut fx = PopupFixture::new();
    fx.offer(&["read", "write"]);
    fx.open_on_the_tool_set();
    fx.press("attendant-properties-pick-right");
    fx.press("attendant-properties-pick-left");

    // When applying.
    fx.press("attendant-properties-apply");

    // Then the filter is unconfigured, so the attendant inherits its
    // parent's growing set again.
    assert!(fx.tool_filter().is_unconfigured());
}

#[rstest::rstest]
#[test]
fn freezing_over_an_empty_capture_writes_nothing() {
    // Given an attendant with no tool registry attached at all.
    let mut fx = PopupFixture::new();
    fx.state
        .tool_registry()
        .expect("registry cell")
        .update(|r| {
            r.global.clear();
        });
    fx.open_on_the_tool_set();

    // When freezing the row.
    fx.press("attendant-properties-pick-right");

    // Then the row stays Live: an allow list over nothing is read as no
    // filter at all, so writing one would look frozen in `jinn.toml` and
    // behave as inheriting.
    assert_eq!(fx.cell.read().set_mode_of(SetField::Tool), SetMode::Live);
    assert!(fx.cell.read().pending_set(SetField::Tool).is_none());
    // And applying writes no empty allow list.
    fx.press("attendant-properties-apply");
    assert!(fx.tool_filter().is_unconfigured());
}

#[rstest::rstest]
#[test]
fn freezing_a_glob_filter_drops_the_glob_and_says_so() {
    // Given an attendant whose tool filter holds a glob.
    let mut fx = PopupFixture::new();
    fx.offer(&["read", "mcp__github__create_pr", "mcp__github__list_prs"]);
    fx.with_tool_filter(jinn_core_types::NameFilter::deny([
        "mcp__github__*".to_owned()
    ]));
    fx.open_on_the_tool_set();

    // When freezing the row.
    fx.press("attendant-properties-pick-right");

    // Then the capture is all literals — the glob is gone, because a
    // pattern means "only" in an allow list and would have inverted.
    let captured = fx.cell.read().pending_set(SetField::Tool).cloned();
    assert_eq!(
        captured,
        Some(allow(&["read"]).names),
        "the glob must not survive into the frozen set"
    );
    // And the status line names the resource and says a pattern was lost.
    assert_eq!(
        fx.cell.read().status,
        Some(PopupStatus::GlobDropped {
            field: SetField::Tool
        })
    );
}

#[rstest::rstest]
#[test]
fn freezing_a_literal_filter_reports_nothing() {
    // Given an attendant whose tool filter is all literals.
    let mut fx = PopupFixture::new();
    fx.offer(&["read", "write", "bash"]);
    fx.with_tool_filter(jinn_core_types::NameFilter::deny([
        "bash".to_owned(),
        "write".to_owned(),
    ]));
    fx.open_on_the_tool_set();

    // When freezing the row.
    fx.press("attendant-properties-pick-right");

    // Then no drop is reported — nothing was lost.
    assert!(fx.cell.read().status.is_none());
}

#[rstest::rstest]
#[test]
fn freezing_the_tool_set_leaves_the_skill_row_live() {
    // Given an open popup.
    let mut fx = PopupFixture::new();
    fx.offer(&["read"]);
    fx.offer_skills(&["web-coder"]);
    fx.open_on_the_tool_set();

    // When freezing the tool set.
    fx.press("attendant-properties-pick-right");

    // Then the skill row is untouched.
    assert_eq!(fx.cell.read().set_mode_of(SetField::Skill), SetMode::Live);
    assert!(fx.cell.read().pending_set(SetField::Skill).is_none());
}

#[rstest::rstest]
#[test]
fn the_set_rows_are_reachable_while_composing() {
    // Given an open popup over an attendant that is still being composed,
    // cursor on the prep row.
    let mut fx = PopupFixture::new();
    fx.open();
    assert!(fx.cell.read().pending_prep_mode);

    // When pressing j twice — off the prep row and onto the skill row.
    fx_press_n(&mut fx, 2);

    // Then the skill set row is reached. The cage above the prep row does
    // not extend below it: a composing attendant still has a tool budget.
    assert_eq!(fx.cell.read().focus, PropertyField::SkillSet);
}
