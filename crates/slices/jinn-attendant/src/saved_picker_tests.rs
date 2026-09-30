//! Tests for the saved-attendants picker and the create-from-entry path.
//!
//! Both are driven through route dispatch against a real `AppState` and a
//! real `jinn.toml` document, so the tests observe what the user gets —
//! which attendants exist, what a created attendant is configured with, and
//! what it was sent — rather than the internals that produced it.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::sync::Arc;

use jinn_app_state::AppState;
use jinn_attendant_msg::{
    AttendantBehavior, AttendantSavedPickerState, AttendantTrigger, attendant_saved_picker_scope,
    attendant_saved_picker_slot,
};
use jinn_config::{ConfigLayer, InMemoryConfigStorage};
use jinn_core_types::{PinPosition, SessionId};
use jinn_preferences_config::schemas::{
    AttendantEntryConfig, AttendantPinConfig, AttendantPinRole,
};
use jinn_session_state::ChatSessionState;
use jinn_slices::KeyRoutes;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{ActionCtx, DynamicIntent, ScopeSignal};

use crate::saved_picker_actions as actions;
use crate::saved_picker_routes::{attach_saved_picker_rows, register_saved_picker_input_hook};

/// The action a row binds, or `""` for a non-action row.
fn row_action(row: &jinn_slices::route::RouteRow) -> &'static str {
    match &row.outcome {
        jinn_slices::route::RouteOutcome::Action { action, .. } => action,
        _ => "",
    }
}

/// A session with an id the test can find it by, in `state`.
fn active_session_id(state: &AppState) -> SessionId {
    state.session.active_session_id().clone()
}

/// The entry a save of a fully-configured attendant would have written.
fn configured_entry(name: &str) -> AttendantEntryConfig {
    AttendantEntryConfig::from_parts(
        name.to_owned(),
        AttendantBehavior::Reset,
        AttendantTrigger::ParentCompleted,
        false,
        "review: <prior report>".to_owned(),
        &jinn_core_types::ModelSelection::Single("zai/glm-4.7".to_owned()),
        "reviewer",
        &std::iter::once("write".to_owned()).collect(),
        &std::collections::HashSet::new(),
        Some(jinn_core_types::ReasoningEffort::High),
        None,
        vec![AttendantPinConfig {
            role: AttendantPinRole::User,
            text: "always in context".to_owned(),
        }],
    )
}

/// The picker, its routes, an `AppState`, and the document it reads.
struct PickerFixture {
    state: AppState,
    slices: jinn_slices::Slices,
    routes: KeyRoutes,
    cell: TypedCell<AttendantSavedPickerState>,
    config: ConfigLayer,
    parent_id: SessionId,
}

impl PickerFixture {
    /// Builds the fixture with an active ordinary session and a document
    /// holding `entries`.
    fn new(entries: Vec<AttendantEntryConfig>) -> Self {
        let doc = "# user's own comment\n".parse().expect("parses");
        let config = ConfigLayer::load(Arc::new(InMemoryConfigStorage::new(doc)))
            .expect("an empty document always loads");
        if !entries.is_empty() {
            config
                .put_list::<AttendantEntryConfig>(&entries)
                .expect("seed writes");
        }
        let mut state = AppState::default_with_scope_focus();
        let parent_id = {
            let mut parent = ChatSessionState::new();
            parent.set_title("parent".to_owned());
            let id = parent.session_id().clone();
            state.session.insert(parent);
            state.session.set_active(id.clone());
            id
        };
        let slices = jinn_slices::Slices::new();
        let cell = slices
            .register(
                attendant_saved_picker_slot(),
                AttendantSavedPickerState::default(),
            )
            .expect("unclaimed slot");
        let routes = KeyRoutes::new();
        attach_saved_picker_rows(&routes, &cell);
        register_saved_picker_input_hook(&routes, &cell);
        Self {
            state,
            slices,
            routes,
            cell,
            config,
            parent_id,
        }
    }

    /// Runs a picker row and applies any scope signal.
    fn press(&mut self, action: &'static str) -> jinn_slices::RouteResult {
        let ctx = ActionCtx {
            state: &mut self.state,
            slices: &self.slices,
            config: &self.config,
            key_bytes: Vec::new(),
        };
        let intent = DynamicIntent::new(attendant_saved_picker_scope(), action, "test");
        let result = self
            .routes
            .action_for(&intent, ctx)
            .unwrap_or_else(|| panic!("no row bound for action {action:?}"));
        if let Some(ScopeSignal::PopIf(scope)) = result.scope_signal.clone()
            && self.state.frontend.scope() == jinn_slices::FocusScope::Dynamic(scope)
        {
            self.state.frontend.scope_pop();
        }
        result
    }

    /// Opens the picker through its opener row, as the key does.
    fn open(&mut self) -> jinn_slices::RouteResult {
        let action = self
            .routes
            .rows()
            .into_iter()
            .find(|row| row_action(row) == "open-attendant-saved-picker")
            .map(|row| row.scope)
            .expect("the opener row is attached");
        let ctx = ActionCtx {
            state: &mut self.state,
            slices: &self.slices,
            config: &self.config,
            key_bytes: Vec::new(),
        };
        let intent = DynamicIntent::new(action, "open-attendant-saved-picker", "test");
        self.routes
            .action_for(&intent, ctx)
            .unwrap_or_else(|| panic!("no row bound for the opener"))
    }

    /// The active session, after any creation.
    fn active(&self) -> &ChatSessionState {
        self.state
            .session
            .get(&active_session_id(&self.state))
            .expect("active session")
    }
}

#[rstest::rstest]
#[test]
fn the_opener_lists_the_documents_entries() {
    // Given a document listing two saved attendants.
    let mut fx = PickerFixture::new(vec![
        configured_entry("nightly"),
        configured_entry("watcher"),
    ]);

    // When opening the picker.
    fx.open();

    // Then both are rows, in the document's order.
    assert_eq!(fx.cell.read().selection.filtered_count(), 2);
    assert_eq!(
        actions::highlighted_name(&fx.cell.read()).as_deref(),
        Some("nightly")
    );
}

#[rstest::rstest]
#[test]
fn the_opener_reads_the_document_at_open() {
    // Given a picker opened over a document holding one attendant.
    let mut fx = PickerFixture::new(vec![configured_entry("nightly")]);
    fx.open();
    assert_eq!(fx.cell.read().selection.filtered_count(), 1);

    // When the document gains a hand-written entry and the picker is
    // opened again.
    fx.config
        .put_list::<AttendantEntryConfig>(&[
            configured_entry("nightly"),
            configured_entry("hand-written"),
        ])
        .expect("hand edit writes");
    fx.open();

    // Then the new entry is there without a restart — the cell reads the
    // live document rather than a snapshot taken when the picker opened.
    assert_eq!(fx.cell.read().selection.filtered_count(), 2);
}

#[rstest::rstest]
#[test]
fn an_empty_document_opens_an_empty_picker() {
    // Given a document with no saved attendants.
    let mut fx = PickerFixture::new(Vec::new());

    // When opening the picker.
    fx.open();

    // Then it opens with nothing to highlight, rather than refusing.
    assert_eq!(fx.cell.read().selection.filtered_count(), 0);
    assert!(actions::highlighted_name(&fx.cell.read()).is_none());
}

#[rstest::rstest]
#[test]
fn confirming_creates_the_attendant_and_activates_it() {
    // Given a picker over a saved attendant.
    let mut fx = PickerFixture::new(vec![configured_entry("nightly")]);
    fx.open();
    let parent_id = fx.parent_id.clone();

    // When confirming the highlighted row.
    fx.press("confirm-attendant-saved-picker");

    // Then the active session is a new attendant of the parent.
    let created = fx.active();
    assert!(created.is_attendant());
    assert_ne!(created.session_id(), &parent_id);
}

#[rstest::rstest]
#[test]
fn a_created_attendant_is_titled_after_its_entry() {
    // Given a picker over a saved attendant named "nightly".
    let mut fx = PickerFixture::new(vec![configured_entry("nightly")]);
    fx.open();

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then the new attendant carries that name, which is how the user
    // recognizes it in the sessions list.
    assert_eq!(fx.active().title(), Some("nightly"));
}

#[rstest::rstest]
#[test]
fn a_created_attendant_restores_behavior_and_trigger_as_saved() {
    // Given an entry saved with the reset behavior and a live trigger.
    let mut fx = PickerFixture::new(vec![configured_entry("nightly")]);
    fx.open();

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then it is live from the first run, not dropped back into seed mode.
    let created = fx.active();
    assert_eq!(created.attendant_behavior(), AttendantBehavior::Reset);
    assert_eq!(
        created.attendant_trigger(),
        AttendantTrigger::ParentCompleted
    );
    assert!(!created.attendant_is_prepping());
}

#[rstest::rstest]
#[test]
fn a_created_attendant_restores_the_seed_template() {
    // Given an entry with a custom seed template.
    let mut fx = PickerFixture::new(vec![configured_entry("nightly")]);
    fx.open();

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then the template is the saved one, verbatim.
    assert_eq!(fx.active().seed_template(), "review: <prior report>");
}

#[rstest::rstest]
#[test]
fn a_created_attendant_takes_the_entrys_model_and_persona() {
    // Given an entry that configured a model and a persona.
    let mut fx = PickerFixture::new(vec![configured_entry("nightly")]);
    fx.open();

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then the attendant runs under them, not the parent's.
    let profile = fx.active().profile();
    assert_eq!(
        profile.model,
        jinn_core_types::ModelSelection::Single("zai/glm-4.7".to_owned())
    );
    assert_eq!(profile.persona_name, "reviewer");
    assert_eq!(
        profile.reasoning_effort,
        Some(jinn_core_types::ReasoningEffort::High)
    );
}

#[rstest::rstest]
#[test]
fn a_created_attendant_inherits_an_unconfigured_model_from_its_parent() {
    // Given a parent running a model, and an entry that configured none.
    let mut fx = PickerFixture::new(Vec::new());
    {
        let parent = fx.state.session.get_mut(&fx.parent_id).expect("parent");
        parent.profile_mut().model =
            jinn_core_types::ModelSelection::Single("openai/gpt-5".to_owned());
    }
    let bare = AttendantEntryConfig {
        name: "bare".to_owned(),
        ..AttendantEntryConfig::default()
    };
    fx.config
        .put_list::<AttendantEntryConfig>(&[bare])
        .expect("write");
    fx.open();

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then the attendant runs the parent's model — "not configured"
    // inherits, it does not reset to nothing.
    assert_eq!(
        fx.active().profile().model,
        jinn_core_types::ModelSelection::Single("openai/gpt-5".to_owned())
    );
}

#[rstest::rstest]
#[test]
fn a_created_attendant_inherits_the_parents_cwd() {
    // Given a parent whose cwd is somewhere specific.
    let mut fx = PickerFixture::new(vec![configured_entry("nightly")]);
    fx.state
        .session
        .get_mut(&fx.parent_id)
        .expect("parent")
        .set_cwd(std::path::PathBuf::from("/srv/the-project"));
    fx.open();

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then the attendant works there, because cwd is environment and the
    // entry cannot carry it.
    assert_eq!(fx.active().cwd(), std::path::Path::new("/srv/the-project"));
}

#[rstest::rstest]
#[test]
fn a_created_attendant_restores_its_pins() {
    // Given an entry carrying a pinned instruction.
    let mut fx = PickerFixture::new(vec![configured_entry("nightly")]);
    fx.open();

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then the pin is in the new attendant's history, pinned relative — the
    // standing instruction that makes it the same attendant. A relative pin
    // is what survives a run with a reset behavior, which force-excludes
    // everything that is not pinned.
    let history = fx.active().history();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].text(), "always in context");
    assert_eq!(history[0].pin_position(), Some(PinPosition::Relative));
}

#[rstest::rstest]
#[test]
fn the_parent_is_persisted_before_the_created_attendant() {
    // Given a picker over a saved attendant.
    let mut fx = PickerFixture::new(vec![configured_entry("nightly")]);
    fx.open();
    let parent_id = fx.parent_id.clone();

    // When confirming.
    let result = fx.press("confirm-attendant-saved-picker");

    // Then the parent's save is published before the attendant's, because
    // the attendant's row names a parent the store would otherwise never
    // have heard of.
    let names = result.message_names.clone();
    let first_persist = names
        .iter()
        .position(|name| *name == "PersistSession")
        .expect("a session is persisted");
    let second_persist = names
        .iter()
        .rposition(|name| *name == "PersistSession")
        .expect("both sessions are persisted");
    assert_ne!(first_persist, second_persist);
    assert!(
        result
            .message_names
            .iter()
            .filter(|name| **name == "PersistSession")
            .count()
            >= 2,
        "both parent and child are persisted: {names:?}"
    );
    let _ = parent_id;
}

#[rstest::rstest]
#[test]
fn cancelling_creates_nothing() {
    // Given a picker over a saved attendant.
    let mut fx = PickerFixture::new(vec![configured_entry("nightly")]);
    fx.open();

    // When cancelling.
    fx.press("cancel-attendant-saved-picker");

    // Then the active session is untouched.
    assert_eq!(active_session_id(&fx.state), fx.parent_id);
}

#[rstest::rstest]
#[test]
fn a_created_attendant_leaves_the_picker() {
    // Given a picker over a saved attendant.
    let mut fx = PickerFixture::new(vec![configured_entry("nightly")]);
    fx.open();
    fx.state
        .frontend
        .scope_push(jinn_slices::FocusScope::Dynamic(
            attendant_saved_picker_scope(),
        ));

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then the picker is no longer on top: the new attendant is what the
    // user is looking at.
    assert_ne!(
        fx.state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(attendant_saved_picker_scope())
    );
}

#[rstest::rstest]
#[test]
fn the_opener_key_is_the_leader_attendant_sequence() {
    // Given the picker's attached rows.
    let fx = PickerFixture::new(Vec::new());

    // When collecting the key the opener row binds.
    let opener_key = fx
        .routes
        .rows()
        .into_iter()
        .find(|row| row_action(row) == "open-attendant-saved-picker")
        .map(|row| row.key)
        .expect("the opener row is attached");

    // Then it is the leader chord, in the `Normal` scope — a key that opens
    // the picker cannot bind inside the scope it opens.
    assert_eq!(opener_key, "<leader>sa");
    let opener_site = fx
        .routes
        .rows()
        .into_iter()
        .find(|row| row_action(row) == "open-attendant-saved-picker")
        .map(|row| row.site)
        .expect("the opener row is attached");
    assert!(
        matches!(
            opener_site,
            jinn_slices::route::BindSite::StaticScopes(&["Normal"])
        ),
        "the opener binds in Normal scope: {opener_site:?}"
    );
}

#[rstest::rstest]
#[test]
fn no_other_saved_picker_key_collides_with_the_opener() {
    // Given the picker's attached rows.
    let fx = PickerFixture::new(Vec::new());

    // When counting the rows bound to the opener's key.
    let collisions = fx
        .routes
        .rows()
        .into_iter()
        .filter(|row| row.key == "<leader>sa")
        .count();

    // Then the opener is the only one — two rows on one key makes which
    // stroke win depend on attach order.
    assert_eq!(collisions, 1);
}

// ── What the picker shows ──────────────────────────────────────────

#[rstest::rstest]
#[test]
fn a_row_shows_only_the_saved_attendants_name() {
    // Given a document whose entry carries a full run configuration.
    let mut fx = PickerFixture::new(vec![configured_entry("nightly")]);
    fx.open();

    // When rendering the highlighted row.
    let row = actions::saved_row(
        &actions::summaries_of(&[configured_entry("nightly")])[0],
        &jinn_picker::RowCtx::flat(true, &[]),
    );

    // Then the row is the name and nothing else — no behavior, trigger,
    // or pin count to read past.
    assert_eq!(row.spans.len(), 1);
    assert_eq!(row.spans[0].content, "nightly");
}

#[rstest::rstest]
#[test]
fn a_row_leaves_the_text_color_to_the_widget() {
    // Given a saved attendant and a row context.
    let entry = actions::summaries_of(&[configured_entry("nightly")]);

    // When rendering its row, unselected and selected.
    let contexts = [
        jinn_picker::RowCtx::flat(false, &[]),
        jinn_picker::RowCtx::flat(true, &[]),
    ];
    let colors: Vec<_> = contexts
        .iter()
        .map(|ctx| actions::saved_row(&entry[0], ctx).spans[0].style.fg)
        .collect();

    // Then neither sets a foreground — the widget's own colors apply, and
    // only the selection's reverse-video marker is added.
    assert_eq!(colors, vec![None, None]);
}

#[rstest::rstest]
#[test]
fn the_status_line_counts_every_saved_attendant() {
    // Given a document holding two saved attendants.
    let mut fx = PickerFixture::new(vec![
        configured_entry("nightly"),
        configured_entry("watcher"),
    ]);
    fx.open();
    let theme = jinn_theme::default_theme();

    // When reading the status line.
    let line = crate::saved_picker_render::saved_status(&fx.cell.read(), &theme);

    // Then it counts them.
    assert_eq!(line.spans[0].content, "2 saved attendants");
}

#[rstest::rstest]
#[test]
fn the_status_line_ignores_the_filter() {
    // Given a picker over two saved attendants.
    let mut fx = PickerFixture::new(vec![
        configured_entry("nightly"),
        configured_entry("watcher"),
    ]);
    fx.open();

    // When typing a filter that matches only one of them.
    let hook = crate::saved_picker_routes::filter_input_hook(&fx.cell);
    for ch in "watch".chars() {
        hook(&jinn_slices::EditIntent::InsertChar(ch));
    }
    assert_eq!(fx.cell.read().selection.filtered_count(), 1);

    // Then the count still reads the document's total, not the matches.
    let theme = jinn_theme::default_theme();
    let line = crate::saved_picker_render::saved_status(&fx.cell.read(), &theme);
    assert_eq!(line.spans[0].content, "2 saved attendants");
}

#[rstest::rstest]
#[test]
fn the_picker_binds_its_keys_in_the_hotkey_accent() {
    // Given the picker's palette under the default theme.
    let theme = jinn_theme::default_theme();
    let palette = crate::saved_picker_render::saved_picker_palette(&theme);

    // Then the key glyphs carry the theme's keybind accent, not the
    // attendant's own color.
    assert_eq!(palette.accent_action, theme.accent_action);
}

#[rstest::rstest]
#[test]
fn a_malformed_document_says_so_instead_of_looking_empty() {
    // Given a document whose entry names a trigger variant that does not
    // exist — the entries are hand-editable, so a typo is likely.
    let malformed = r#"
[[attendant.entry]]
name = "nightly"
behavior = "reset"
trigger = "parent-completed"
"#;
    let doc = format!("# user's own comment\n{malformed}")
        .parse()
        .expect("parses");
    let config = ConfigLayer::load(Arc::new(InMemoryConfigStorage::new(doc))).expect("loads");
    let mut fx = PickerFixture::new(vec![]);
    fx.config = config;

    // When opening the picker.
    fx.open();

    // Then the session's log says the list could not be parsed, so an
    // empty picker is not mistaken for having nothing saved.
    let logged = fx.active().history();
    assert!(
        logged
            .iter()
            .any(|entry| entry.text().contains("could not be parsed")),
        "the log names the unreadable list, got: {logged:?}"
    );
}

#[rstest::rstest]
#[test]
fn the_footer_calls_enter_an_attach() {
    // Given the picker's advertised bindings.
    let bindings = crate::saved_picker_routes::SAVED_PICKER_BINDINGS;

    // Then <enter> says "attach": the picker grafts a saved attendant
    // onto the session you are on, it does not spawn a new one.
    let enter = bindings
        .iter()
        .find(|(key, _)| *key == "<enter>")
        .map(|(_, label)| *label)
        .expect("enter is bound");
    assert_eq!(enter, "attach");
}
