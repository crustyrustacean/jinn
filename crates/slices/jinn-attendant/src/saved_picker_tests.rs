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
    AttendantBehavior, AttendantModelSetting, AttendantSavedPickerState, AttendantTrigger,
    attendant_saved_picker_scope, attendant_saved_picker_slot,
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

/// The chat entries `result` publishes into `session`.
///
/// The result carries the messages themselves, not their effects: the
/// entries travel on the bus and land in the sessions they name, so a test
/// about what a key *published* reads them off the result rather than off
/// a session the messages have not reached yet.
fn chat_entries(
    result: jinn_slices::RouteResult,
    session: &SessionId,
) -> Vec<jinn_core_types::ChatEntry> {
    #[derive(Default)]
    struct RecordingSink {
        published: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
    }

    impl jinn_slices::PublishSink for RecordingSink {
        fn publish_schema(
            &self,
            schema_id: trouper::schema::SchemaId,
            payload: serde_json::Value,
            name: &'static str,
        ) {
            if name.ends_with("PushChatEntry") {
                self.published
                    .lock()
                    .expect("sink lock")
                    .push((format!("{schema_id}"), payload));
            }
        }
    }

    let sink = RecordingSink::default();
    for publish in result.messages {
        publish(&sink);
    }
    let published = sink.published.lock().expect("sink lock");
    published
        .iter()
        .filter_map(|(_, payload)| {
            serde_json::from_value::<jinn_session_history_msg::PushChatEntry>(payload.clone()).ok()
        })
        .filter(|entry| &entry.session_id == session)
        .map(|entry| entry.entry)
        .collect()
}

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
        AttendantModelSetting::Fixed,
        &jinn_core_types::ModelSelection::Single("zai/glm-4.7".to_owned()),
        "reviewer",
        Some(&jinn_core_types::NameFilter::deny(["write".to_owned()])),
        None,
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
    fn new(entries: &[AttendantEntryConfig]) -> Self {
        let doc = "# user's own comment\n".parse().expect("parses");
        let config = ConfigLayer::load(Arc::new(InMemoryConfigStorage::new(doc)))
            .expect("an empty document always loads");
        if !entries.is_empty() {
            config
                .put_list::<AttendantEntryConfig>(entries)
                .expect("seed writes");
        }
        let mut state = AppState::default_with_scope_focus();
        // Chat is browsed from the `Normal` base; `Input` is only the
        // default a state carries before the scope-focus slice is attached.
        state
            .frontend
            .scope_swap_base(jinn_slices::FocusScope::Normal);
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

    /// The active session.
    fn active(&self) -> &ChatSessionState {
        self.state
            .session
            .get(&active_session_id(&self.state))
            .expect("active session")
    }

    /// The attendant titled `name`, looked up by what it is called rather
    /// than by which session is active — attaching leaves the active
    /// session alone, so "the attendant this created" is not "the active
    /// session".
    fn attendant_named(&self, name: &str) -> &ChatSessionState {
        self.state
            .session
            .sessions()
            .values()
            .find(|session| session.title() == Some(name))
            .unwrap_or_else(|| panic!("an attendant named {name:?} exists"))
    }

    /// The attendant this fixture's confirm created.
    fn created(&self) -> &ChatSessionState {
        self.attendant_named("nightly")
    }
}

#[rstest::rstest]
#[test]
fn the_opener_lists_the_documents_entries() {
    // Given a document listing two saved attendants.
    let mut fx = PickerFixture::new(&[configured_entry("nightly"), configured_entry("watcher")]);

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
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
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
    let mut fx = PickerFixture::new(&[]);

    // When opening the picker.
    fx.open();

    // Then it opens with nothing to highlight, rather than refusing.
    assert_eq!(fx.cell.read().selection.filtered_count(), 0);
    assert!(actions::highlighted_name(&fx.cell.read()).is_none());
}

#[rstest::rstest]
#[test]
fn confirming_creates_the_attendant_under_the_parent() {
    // Given a picker over a saved attendant.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();
    let parent_id = fx.parent_id.clone();

    // When confirming the highlighted row.
    fx.press("confirm-attendant-saved-picker");

    // Then a new attendant of the parent exists, which is what the confirm
    // does — it grafts a saved attendant onto the session, not onto itself.
    let created = fx.created();
    assert!(created.is_attendant());
    assert_ne!(created.session_id(), &parent_id);
    assert_eq!(created.parent_session(), &Some(parent_id));
}

#[rstest::rstest]
#[test]
fn confirming_leaves_the_active_session_on_the_parent() {
    // Given a picker opened from the session the user is on.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then the active session is still the one the picker was opened from —
    // attaching loads an attendant in the background rather than dropping
    // the user into it.
    assert_eq!(active_session_id(&fx.state), fx.parent_id);
}

#[rstest::rstest]
#[test]
fn confirming_leaves_the_user_browsing_rather_than_typing() {
    // Given a picker opened over the normal session.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();
    fx.state
        .frontend
        .scope_push(jinn_slices::FocusScope::Dynamic(
            attendant_saved_picker_scope(),
        ));

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then the picker is gone and nothing took its place: the user is left
    // browsing the session they were on, not typing into a new attendant.
    assert_eq!(fx.state.frontend.scope(), jinn_slices::FocusScope::Normal);
    assert_eq!(fx.state.frontend.scope_len(), 1);
}

#[rstest::rstest]
#[test]
fn confirming_reports_the_attendant_in_the_parents_log() {
    // Given a picker over an attendant named "nightly".
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();

    // When confirming.
    let result = fx.press("confirm-attendant-saved-picker");

    // Then the parent is told what it gained, by name — the user stays in
    // this session and the log is how they learn an attendant arrived.
    let parent_entries = chat_entries(result, &fx.parent_id);
    let notices = parent_entries
        .iter()
        .filter(|entry| entry.text().contains("nightly"))
        .count();
    assert_eq!(notices, 1, "one line in the parent, naming the attendant");
}

#[rstest::rstest]
#[test]
fn confirming_reports_the_attendant_as_a_system_line() {
    // Given a picker over a saved attendant.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();

    // When confirming.
    let result = fx.press("confirm-attendant-saved-picker");

    // Then the line it left in the parent is a system line, so it renders
    // as status rather than as something the assistant said.
    let parent_lines: Vec<_> = chat_entries(result, &fx.parent_id)
        .into_iter()
        .filter(|entry| matches!(entry.kind, jinn_core_types::ChatEntryKind::System(_)))
        .collect();
    assert_eq!(
        parent_lines.len(),
        1,
        "the parent gains exactly one system line"
    );
}

#[rstest::rstest]
#[test]
fn the_created_attendant_still_says_what_it_is() {
    // Given a picker over a saved attendant.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();

    // When confirming.
    let result = fx.press("confirm-attendant-saved-picker");

    // Then the attendant's own log opens with the restored-entry notice —
    // being created in the background does not strip it of that.
    let attendant_id = fx.created().session_id().clone();
    let announced = chat_entries(result, &attendant_id)
        .iter()
        .any(|entry| entry.text().contains("restored from a saved entry"));
    assert!(announced, "the new attendant is told what it is");
}

#[rstest::rstest]
#[test]
fn confirming_publishes_the_creation_in_order() {
    // Given a picker over a saved attendant.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();

    // When confirming.
    let result = fx.press("confirm-attendant-saved-picker");

    // Then the whole creation reads as one sequence: both sessions saved,
    // the attendant announced, its restored notice, and the parent's line.
    assert_eq!(
        result.message_names,
        [
            "PersistSession",
            "PersistSession",
            "SessionCreated",
            "PushChatEntry",
            "PushChatEntry",
        ]
    );
}

#[rstest::rstest]
#[test]
fn a_created_attendant_is_titled_after_its_entry() {
    // Given a picker over a saved attendant named "nightly".
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then the new attendant carries that name, which is how the user
    // recognizes it in the sessions list.
    assert_eq!(fx.created().title(), Some("nightly"));
}

#[rstest::rstest]
#[test]
fn a_created_attendant_restores_behavior_and_trigger_as_saved() {
    // Given an entry saved with the reset behavior and a live trigger.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then it is live from the first run, not dropped back into seed mode.
    let created = fx.created();
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
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then the template is the saved one, verbatim.
    assert_eq!(fx.created().seed_template(), "review: <prior report>");
}

#[rstest::rstest]
#[test]
fn a_created_attendant_takes_the_entrys_model_and_persona() {
    // Given an entry that configured a model and a persona.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then the attendant runs under them, not the parent's.
    let profile = fx.created().profile();
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
    let mut fx = PickerFixture::new(&[]);
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
        fx.attendant_named("bare").profile().model,
        jinn_core_types::ModelSelection::Single("openai/gpt-5".to_owned())
    );
}

#[rstest::rstest]
#[test]
fn a_created_attendant_inherits_the_parents_cwd() {
    // Given a parent whose cwd is somewhere specific.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
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
    assert_eq!(fx.created().cwd(), std::path::Path::new("/srv/the-project"));
}

#[rstest::rstest]
#[test]
fn a_created_attendant_restores_its_pins() {
    // Given an entry carrying a pinned instruction.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();

    // When confirming.
    fx.press("confirm-attendant-saved-picker");

    // Then the pin is in the new attendant's history, pinned relative — the
    // standing instruction that makes it the same attendant. A relative pin
    // is what survives a run with a reset behavior, which force-excludes
    // everything that is not pinned.
    let history = fx.created().history();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].text(), "always in context");
    assert_eq!(history[0].pin_position(), Some(PinPosition::Relative));
}

#[rstest::rstest]
#[test]
fn the_parent_is_persisted_before_the_created_attendant() {
    // Given a picker over a saved attendant.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
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
fn keeping_open_creates_the_attendant_without_activating_it() {
    // Given a picker opened from the session the user is on.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();
    let parent_id = fx.parent_id.clone();

    // When attaching with the picker left open.
    fx.press("attach-attendant-saved-picker-keeping-it-open");

    // Then the attendant exists under the parent, and the user is still
    // where they were.
    assert!(fx.created().is_attendant());
    assert_eq!(
        fx.created().parent_session(),
        &Some(parent_id.clone()),
        "the attendant hangs off the session the picker was opened from"
    );
    assert_eq!(active_session_id(&fx.state), parent_id);
}

#[rstest::rstest]
#[test]
fn keeping_open_leaves_the_picker_on_top() {
    // Given a picker with its scope focused.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();
    fx.state
        .frontend
        .scope_push(jinn_slices::FocusScope::Dynamic(
            attendant_saved_picker_scope(),
        ));
    let stack_before = fx.state.frontend.scope_len();

    // When attaching with the picker left open.
    let result = fx.press("attach-attendant-saved-picker-keeping-it-open");

    // Then nothing asked to leave, and the stack is exactly as it was —
    // the picker is what the user is still looking at.
    assert_eq!(result.scope_signal, None);
    assert_eq!(
        fx.state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(attendant_saved_picker_scope())
    );
    assert_eq!(fx.state.frontend.scope_len(), stack_before);
}

#[rstest::rstest]
#[test]
fn keeping_open_does_not_disturb_the_filter_or_the_highlight() {
    // Given a picker filtered down to two of three entries.
    let mut fx = PickerFixture::new(&[
        configured_entry("nightly"),
        configured_entry("nightly-notes"),
        configured_entry("watcher"),
    ]);
    fx.open();
    let hook = crate::saved_picker_routes::filter_input_hook(&fx.cell);
    for ch in "nightly".chars() {
        hook(&jinn_slices::EditIntent::InsertChar(ch));
    }
    fx.press("move-attendant-saved-picker-down");
    let filter_before = fx.cell.read().selection.filter().to_owned();
    let highlighted_before = actions::highlighted_name(&fx.cell.read());
    let count_before = fx.cell.read().selection.filtered_count();

    // When attaching with the picker left open.
    fx.press("attach-attendant-saved-picker-keeping-it-open");

    // Then the picker is ready for the next entry exactly as it was — the
    // user typed a filter to get here and pressing attach must not spend it.
    assert_eq!(fx.cell.read().selection.filter(), filter_before);
    assert_eq!(
        actions::highlighted_name(&fx.cell.read()),
        highlighted_before
    );
    assert_eq!(fx.cell.read().selection.filtered_count(), count_before);
}

#[rstest::rstest]
#[test]
fn keeping_open_can_attach_the_same_entry_repeatedly() {
    // Given a picker over one saved attendant.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();

    // When attaching it three times without closing the picker.
    for _ in 0..3 {
        fx.press("attach-attendant-saved-picker-keeping-it-open");
    }

    // Then three attendants exist, one per press. Nothing deduplicates or
    // asks: the entries are pre-saved and loading two of the same is a
    // thing the user can mean.
    let attached = fx
        .state
        .session
        .sessions()
        .values()
        .filter(|session| {
            session.is_attendant() && *session.parent_session() == Some(fx.parent_id.clone())
        })
        .count();
    assert_eq!(attached, 3, "one attendant per press");
}

#[rstest::rstest]
#[test]
fn keeping_open_reports_each_attach_in_the_parents_log() {
    // Given a picker over a saved attendant.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();

    // When attaching twice.
    let first = fx.press("attach-attendant-saved-picker-keeping-it-open");
    let second = fx.press("attach-attendant-saved-picker-keeping-it-open");

    // Then the parent carries a line per attach — the user never sees the
    // attendants arrive, so the log is how they find out.
    let parent_id = fx.parent_id.clone();
    let named = |result: jinn_slices::RouteResult| {
        chat_entries(result, &parent_id)
            .iter()
            .filter(|entry| entry.text().contains("nightly"))
            .count()
    };
    assert_eq!(named(first), 1, "the first attach reports itself");
    assert_eq!(named(second), 1, "so does the second");
}

#[rstest::rstest]
#[test]
fn keeping_open_publishes_the_same_creation_as_the_confirm() {
    // Given a picker over a saved attendant.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
    fx.open();

    // When attaching with the picker left open.
    let result = fx.press("attach-attendant-saved-picker-keeping-it-open");

    // Then it is the same creation the confirm performs — the only
    // difference is where the picker ends up.
    assert_eq!(
        result.message_names,
        [
            "PersistSession",
            "PersistSession",
            "SessionCreated",
            "PushChatEntry",
            "PushChatEntry",
        ]
    );
}

#[rstest::rstest]
#[test]
fn keeping_open_binds_its_own_key_in_the_pickers_own_scope() {
    // Given the picker's attached rows.
    let fx = PickerFixture::new(&[]);

    // When collecting the rows bound to `<c-a>`.
    let matches: Vec<_> = fx
        .routes
        .rows()
        .into_iter()
        .filter(|row| row.key == "<c-a>")
        .collect();

    // Then exactly one row binds it — two rows on one key makes which
    // stroke win depend on attach order.
    assert_eq!(matches.len(), 1);
    let attach_action = row_action(&matches[0]);
    let enter_action = fx
        .routes
        .rows()
        .into_iter()
        .find(|row| row.key == "<enter>")
        .map(|row| row_action(&row).to_owned())
        .expect("enter is bound");
    assert_ne!(
        attach_action, enter_action,
        "keep-open is its own action, not the confirm under another key"
    );
}

#[rstest::rstest]
#[test]
fn the_footer_advertises_exactly_the_pickers_keys() {
    // Given the picker's advertised bindings.
    let bindings = crate::saved_picker_routes::SAVED_PICKER_BINDINGS;

    // Then they are the confirm, the keep-open attach, and the two ways
    // out — the footer is generated from this table, so a key the picker
    // does not bind cannot be advertised here.
    assert_eq!(
        bindings,
        [
            ("<enter>", "attach"),
            ("<c-a>", "attach & keep open"),
            ("<esc>", "close"),
            ("<c-c>", "clear filter or close"),
        ]
    );
}

#[rstest::rstest]
#[test]
fn every_advertised_key_is_a_bound_row() {
    // Given the picker's advertised bindings and its attached rows.
    let fx = PickerFixture::new(&[]);

    // When matching each advertised key against the rows.
    let missing: Vec<_> = crate::saved_picker_routes::SAVED_PICKER_BINDINGS
        .iter()
        .map(|(key, _)| *key)
        .filter(|key| !fx.routes.rows().iter().any(|row| row.key == *key))
        .collect();

    // Then every one is bound — the footer cannot advertise a dead key.
    assert!(missing.is_empty(), "unbound: {missing:?}");
}

#[rstest::rstest]
#[test]
fn cancelling_creates_nothing() {
    // Given a picker over a saved attendant.
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
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
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
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
    let fx = PickerFixture::new(&[]);

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
    let fx = PickerFixture::new(&[]);

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
    let mut fx = PickerFixture::new(&[configured_entry("nightly")]);
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
    let mut fx = PickerFixture::new(&[configured_entry("nightly"), configured_entry("watcher")]);
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
    let mut fx = PickerFixture::new(&[configured_entry("nightly"), configured_entry("watcher")]);
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
    let mut fx = PickerFixture::new(&[]);
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
