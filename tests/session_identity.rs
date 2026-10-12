//! Session-identity entry tests.
//!
//! Every session-creation surface — plain new session, scripted lifecycle,
//! picker-attached attendant, `load_attendant` cast member (both through
//! `saved_create::creation_messages`), and the `task` tool's subagent
//! child — must publish the same identity `PushChatEntry`: a pinned `Top`
//! system entry naming that session's own id, forced into the LLM
//! context. The boot session is covered source-level in
//! `src/actor_wiring.rs`, whose startup tail publishes for
//! `initial_session_id` directly on the bus (the boot_list tests treat
//! that file as the composition source of truth the same way).
//!
//! These live here rather than in each slice because the invariant is
//! "no path that publishes `SessionCreated` publishes it without the
//! identity push" — a statement about all the paths at once, over four
//! crates no one of them can see.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use std::time::Duration;

use jinn_app_state::AppState;
use jinn_kernel::common::state::State;
use jinn_kernel::protocol::{ChatEntry, PinPosition};
use jinn_session_history_msg::PushChatEntry;
use jinn_session_msg::{PhaseKind, SessionPhaseChanged};
use jinn_session_state::ChatSessionState;
use jinn_slices::route_publish::PublishSink;
use jinn_testutil::bus_harness::await_recorded;

/// The recorder wait budget — the task tool's own tests use the same.
const AWAIT_TIMEOUT: Duration = Duration::from_secs(3);

/// Drains a creation's `IntentResult` through a recording sink and
/// returns every `PushChatEntry` payload decoded.
fn pushed_entries(result: jinn_slices::RouteResult) -> Vec<PushChatEntry> {
    let sink = RecordingSink::default();
    for closure in result.messages {
        closure(&sink);
    }
    sink.published
        .lock()
        .expect("sink lock")
        .iter()
        .filter(|(id, _)| id.ends_with("PushChatEntry"))
        .filter_map(|(_, payload)| serde_json::from_value(payload.clone()).ok())
        .collect()
}

/// The one assertion the invariant is: among the pushes, the target
/// session received the identity entry — pinned Top, a system entry
/// naming its own id, forced into the LLM context.
fn assert_identity_entry(pushes: &[PushChatEntry], session_id: &jinn_core_types::SessionId) {
    let identity = pushes
        .iter()
        .find(|push| push.session_id == *session_id && push.pin == Some(PinPosition::Top))
        .unwrap_or_else(|| {
            panic!(
                "{session_id} must receive an identity entry pinned Top; pushes: {}",
                pushes.len()
            )
        });
    assert!(
        matches!(
            &identity.entry.kind,
            jinn_core_types::chat_entry::ChatEntryKind::System(text)
                if text.contains(&session_id.to_string())
        ),
        "the identity entry is a system entry naming the session's own id"
    );
    assert_eq!(
        identity.entry.context_override(),
        jinn_core_types::ContextOverride::ForcedInclude,
        "the identity is forced into the LLM context, so it survives compaction"
    );
}

/// A publish sink that keeps every payload published through it.
#[derive(Default)]
struct RecordingSink {
    published: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
}

impl PublishSink for RecordingSink {
    fn publish_schema(
        &self,
        schema_id: trouper::schema::SchemaId,
        payload: serde_json::Value,
        name: &'static str,
    ) {
        let _ = name;
        self.published
            .lock()
            .expect("sink lock")
            .push((format!("{schema_id}"), payload));
    }
}

#[rstest::rstest]
#[test]
fn plain_new_session_publishes_the_identity_entry() {
    // Given a single empty session (app just started).
    let mut state = AppState::default_with_scope_focus();

    // When creating a new session with no lifecycle setup command.
    let result = jinn_app_state::session_creation::intent::handle_session_lifecycle_setup(
        &mut state,
        "",
        &[],
        None,
        jinn_config::empty_config_layer(),
    );

    // Then the identity entry rides the result for the new session.
    let new_id = state.active_session().session_id().clone();
    assert_identity_entry(&pushed_entries(result), &new_id);
}

#[rstest::rstest]
#[test]
fn scripted_lifecycle_publishes_the_identity_entry() {
    // Given a session store whose active session runs a lifecycle with a
    // setup command — the scripted branch that returns early.
    let mut state = AppState::default_with_scope_focus();
    state
        .active_session_mut()
        .set_lifecycle_name(Some("smoke".to_owned()));

    // When creating a new session under that lifecycle.
    let config = jinn_config::testutil::config_layer(
        "[[session_lifecycle.script]]\nname = \"smoke\"\nsetup_command = \"echo hello\"\n",
    );
    let result = jinn_app_state::session_creation::intent::handle_session_lifecycle_setup(
        &mut state,
        "smoke",
        &[],
        None,
        &config,
    );

    // Then the identity entry rides the result for the new session — the
    // scripted branch must not be the only one, but it must not lose it
    // either.
    let new_id = state.active_session().session_id().clone();
    assert_identity_entry(&pushed_entries(result), &new_id);
}

#[rstest::rstest]
#[test]
fn picker_attendant_and_loaded_cast_member_publish_the_identity_entry() {
    // Given a parent and an attendant built the way the picker's save
    // path and `load_attendant` both build them.
    let parent = ChatSessionState::new();
    let entry = jinn_preferences_config::schemas::AttendantEntryConfig {
        name: "cast-member".to_owned(),
        ..jinn_preferences_config::schemas::AttendantEntryConfig::default()
    };
    let mut attendant = jinn_attendant::saved_create::build(&entry, &parent);
    // The load_attendant stamp, on top of the same build path.
    attendant.set_workflow_cast("res/workflows/autocoder/attendants/echo.toml".to_owned());

    // When the creation's messages are built.
    let pushes = pushed_entries(jinn_attendant::saved_create::creation_messages(
        parent.session_id(),
        &attendant,
    ));

    // Then the attendant received the identity entry.
    let attendant_id = attendant.session_id().clone();
    assert_identity_entry(&pushes, &attendant_id);
}

#[rstest::rstest]
#[tokio::test]
async fn task_tool_child_publishes_the_identity_entry() {
    // Given a parent session, the task tool's test wiring, and a recorder
    // for the history pushes — the same harness shape the tool's own
    // tests use.
    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let state = State::new(AppState::default_with_scope_focus());
    let parent_id = jinn_core_types::SessionId::new();
    state.write().session.get_or_create(&parent_id);
    let services = jinn_kernel::common::bus::HarnessServices::services(&harness).await;
    let ctx = jinn_tools::tool_types::ToolContext {
        cwd: std::path::PathBuf::from("/tmp"),
        config: jinn_config::testutil::config_layer(""),
        timeout: None,
        state: Some(state.clone()),
        session_id: Some(parent_id),
        app_paths: jinn_kernel::AppPaths::new_in(std::path::Path::new("/tmp")),
        bus: Some(harness.bus()),
        max_output_lines: None,
        max_output_bytes: None,
        dispatched_at: jiff::Timestamp::now(),
        mcp_coordinator: None,
        interactive_term: None,
        task_spawns: Some(services.task_spawns.clone()),
        session_store: Some(services.session_store.clone()),
        trouper_system: Some(harness.system().clone()),
        model_name_resolver: None,
    };
    let push_rec = harness
        .spawn_recorder::<jinn_session_history_msg::PushChatEntry>()
        .await;
    let created_rec = harness
        .spawn_recorder::<jinn_session_lifecycle_msg::SessionCreated>()
        .await;

    // When executing a task call and driving the child to a finished turn
    // the way the session actor would.
    let pending = tokio::spawn(jinn_tools::task::execute(
        jinn_core_types::tool_types::ToolCall {
            id: "tc_identity".to_owned(),
            name: "task".to_owned(),
            arguments: r#"{"prompt": "Explore."}"#.to_owned(),
        },
        ctx,
    ));
    let created = await_recorded(&created_rec, 1, AWAIT_TIMEOUT).await;
    let child_id = created[0].session_id.clone();
    settle_child(&harness, &child_id).await;
    finish_child(&harness.bus(), &state, &child_id).await;
    let _ = pending.await;

    // Then the child received the identity entry.
    let pushes = await_recorded(&push_rec, 1, AWAIT_TIMEOUT).await;
    assert_identity_entry(&pushes, &child_id);
}

/// The discovery `*Loaded` events the settle gate waits on, all clean.
async fn settle_child(
    harness: &jinn_testutil::bus_harness::TestHarness,
    child_id: &jinn_core_types::SessionId,
) {
    let bus = harness.bus();
    bus.publish(jinn_session_init_msg::ContextFilesLoaded {
        session_id: child_id.clone(),
        files: vec![],
        error: None,
    })
    .await;
    bus.publish(jinn_skills_msg::SkillsLoaded {
        session_id: child_id.clone(),
        skills: vec![],
        error: None,
    })
    .await;
    bus.publish(jinn_session_init_msg::PromptTemplatesLoaded {
        session_id: child_id.clone(),
        templates: vec![],
        error: None,
    })
    .await;
}

/// Drives the child to a finished assistant turn — the Sending → Streaming
/// → Idle sequence a real child run produces, folded into state the same
/// way the session and phase actors fold it, then signaled with the
/// `Idle` phase event.
async fn finish_child(
    bus: &jinn_kernel::common::services::bus_service::BusService,
    state: &State,
    child_id: &jinn_core_types::SessionId,
) {
    let old = {
        let mut w = state.write();
        let child = w.session.get_mut(child_id).expect("child seeded");
        let old = child.phase();
        child.begin_sending();
        child.begin_streaming();
        child.push_entry(ChatEntry::assistant("Done."));
        child.finalize_entries_for_finish(false, jiff::Timestamp::now());
        child.finish_streaming_via_machine();
        old
    };
    bus.publish(SessionPhaseChanged {
        session_id: child_id.clone(),
        old_phase: old,
        new_phase: PhaseKind::Idle,
        at: jiff::Timestamp::now(),
    })
    .await;
}

#[rstest::rstest]
#[test]
fn the_identity_sentence_is_the_same_on_every_surface() {
    // Given the shared constructor.
    let session_id = jinn_core_types::SessionId::new();

    // When building the entry.
    let entry = jinn_core_types::ChatEntry::session_identity(&session_id);

    // Then it carries the one sentence — the thing every surface composes,
    // so no path can drift into its own wording.
    let jinn_core_types::chat_entry::ChatEntryKind::System(text) = entry.kind else {
        panic!("the identity entry is a system entry");
    };
    assert_eq!(
        text,
        format!(
            "This session's id is {session_id}. Address messages to this session with \
             message_session using this id."
        )
    );
}
