//! Tests for the pure run-preparation helpers.

#![allow(clippy::expect_used, reason = "test code")]

use jinn_attendant_msg::{PRIOR_REPORT_PLACEHOLDER, default_seed_template};
use jinn_core_types::chat_entry::ChatEntry;
use jinn_core_types::{ContextOverride, PinPosition};
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::common::bus::HarnessServices;
use jinn_kernel::common::state::State;
use jinn_session_msg::SessionOrigin;
use jinn_session_state::ChatSessionState;

use crate::activation::{prepare_run, render_seed_text, reset_context};

#[rstest::rstest]
#[case("check: <prior report>", Some("the build was green"), "check: the build was green")]
#[case("fixed prompt", Some("the build was green"), "fixed prompt\n\nthe build was green")]
#[case("check: <prior report>", None, "check: <prior report>")]
#[case("fixed prompt", None, "fixed prompt")]
fn seed_rendering_substitutes_or_appends(
    #[case] template: &str,
    #[case] prior: Option<&str>,
    #[case] expected: &str,
) {
    // Given a seed template and whatever the prior run reported.

    // When the run's seed text is rendered.
    let rendered = render_seed_text(template, prior);

    // Then the placeholder is substituted, or the report is appended when
    // the template has none.
    assert_eq!(rendered.as_deref(), Some(expected));
}

#[rstest::rstest]
#[test]
fn empty_template_injects_nothing() {
    // Given an attendant whose user cleared the seed template.

    // When the seed text is rendered, with and without a prior report.
    let without_prior = render_seed_text("", None);
    let with_prior = render_seed_text("", Some("a finding"));

    // Then neither produces a seed — an empty template means no injection.
    assert_eq!(without_prior, None);
    assert_eq!(with_prior, None);
}

#[rstest::rstest]
#[test]
fn default_template_carries_the_placeholder() {
    // Given the shipped default template.

    // When it is rendered against a prior report.
    let rendered = render_seed_text(&default_seed_template(), Some("finding"));

    // Then the placeholder was substituted — the default is usable as-is.
    assert!(rendered.is_some_and(|text| !text.contains(PRIOR_REPORT_PLACEHOLDER)));
}

/// Builds a session with one pinned and two unpinned entries, returning
/// (session, pinned_id, unpinned_ids).
fn session_with_pins() -> (ChatSessionState, jinn_core_types::ChatEntryId, Vec<jinn_core_types::ChatEntryId>) {
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("pinned instructions"));
    let pinned_id = session.history()[0].id.clone();
    session.pin_entry(&pinned_id, PinPosition::Relative);
    session.push_entry(ChatEntry::assistant("an answer"));
    session.push_entry(ChatEntry::user("a follow-up"));
    let unpinned: Vec<_> = session.history()[1..]
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    (session, pinned_id, unpinned)
}

#[rstest::rstest]
#[test]
fn reset_context_excludes_every_non_pinned_entry() {
    // Given a session with one pinned and two unpinned entries.
    let (mut session, pinned_id, unpinned) = session_with_pins();

    // When the context is reset.
    let changed = reset_context(&mut session);

    // Then only the unpinned entries were excluded — the model will see
    // exactly the pins.
    assert_eq!(changed.len(), 2);
    assert!(changed.contains(&unpinned[0]));
    assert!(changed.contains(&unpinned[1]));
    let history = session.history();
    assert_ne!(
        history[0].context_override(),
        ContextOverride::ForcedExclude,
        "the pinned entry must survive the reset"
    );
    assert_eq!(history[1].context_override(), ContextOverride::ForcedExclude);
    assert_eq!(history[2].context_override(), ContextOverride::ForcedExclude);
    assert_eq!(history[0].id, pinned_id);
}

#[rstest::rstest]
#[test]
fn reset_context_is_idempotent() {
    // Given a session whose context has already been reset once.
    let (mut session, _pinned, _unpinned) = session_with_pins();
    reset_context(&mut session);

    // When the context is reset again.
    let changed = reset_context(&mut session);

    // Then nothing changes — already-excluded entries are no-ops.
    assert!(changed.is_empty());
}

#[rstest::rstest]
#[test]
fn continue_activation_prepares_no_entry() {
    // Given a reset attendant with a prior report, switched to continue mode.
    let mut parent = ChatSessionState::new();
    let mut session = ChatSessionState::new_attendant(&parent, true);
    session.set_attendant_activation(jinn_attendant_msg::AttendantActivation::Continue);
    session.append_attendant_report("a finding".to_owned());

    // When the run is prepared.
    let seed = prepare_run(&session);

    // Then nothing is injected — the existing conversation carries the run.
    assert!(seed.is_none());
}

#[rstest::rstest]
#[test]
fn reset_run_seeds_through_the_template_with_the_prior_report() {
    // Given a reset attendant that reported once.
    let mut parent = ChatSessionState::new();
    let mut session = ChatSessionState::new_attendant(&parent, true);
    session.set_attendant_activation(jinn_attendant_msg::AttendantActivation::Reset);
    session.append_attendant_report("the tests were actually passing".to_owned());

    // When the run is prepared.
    let seed = prepare_run(&session).expect("reset mode with a prior report seeds");

    // Then the seed entry carries the report through the template.
    let jinn_core_types::chat_entry::ChatEntryKind::User { display, .. } = &seed.kind else {
        panic!("seed must be a user entry");
    };
    assert!(display.contains("the tests were actually passing"));
    assert!(!display.contains(PRIOR_REPORT_PLACEHOLDER));
}

#[rstest::rstest]
#[test]
fn attendant_created_from_parent_links_and_defaults() {
    // Given a parent with environment values.
    let mut parent = ChatSessionState::new();
    parent.set_project(Some(std::path::PathBuf::from("/tmp/p")));

    // When an attendant is created from it.
    let attendant = ChatSessionState::new_attendant(&parent, true);

    // Then the attendant links the parent with seed activation.
    assert_eq!(attendant.origin(), SessionOrigin::Attendant);
    assert_eq!(
        attendant.attendant_activation(),
        jinn_attendant_msg::AttendantActivation::Seed
    );
    assert_eq!(attendant.project(), Some(std::path::Path::new("/tmp/p")));
    assert!(attendant.is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn seed_activation_makes_the_trigger_inert_end_to_end() {
    // Given a parent with a ParentCompleted attendant still in seed mode,
    // and the trigger actor live on the bus.
    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let dispatched = harness
        .spawn_recorder::<jinn_chat_input_msg::EnqueueUserMessage>()
        .await;
    let canceled = harness
        .spawn_recorder::<jinn_inference_msg::CancelStream>()
        .await;
    let state = State::new(AppState::default());
    {
        let mut s = state.write();
        let parent = ChatSessionState::new();
        let parent_id = parent.session_id().clone();
        s.session.insert(parent);
        let mut attendant = {
            let read = s.session.get(&parent_id).expect("parent").clone();
            ChatSessionState::new_attendant(&read, true)
        };
        attendant.set_attendant_trigger(jinn_attendant_msg::AttendantTrigger::ParentCompleted);
        // Still in Seed activation — the user has not armed it.
        attendant.set_seed_template("re-check".to_owned());
        s.session.insert(attendant);
    }
    let _actor = crate::trigger_actor::AttendantTriggerActor::spawn(
        harness.system(),
        crate::trigger_actor::AttendantTriggerActorDeps {
            services: harness.services().await,
            state: state.clone(),
        },
    );

    // When the parent's turn completes successfully.
    let parent_id = {
        let s = state.read();
        s.session
            .iter()
            .find(|(_, sess)| !sess.is_attendant())
            .map(|(id, _)| id.clone())
            .expect("parent")
    };
    harness
        .publish(jinn_session_msg::TurnCompleted {
            session_id: parent_id,
            outcome: jinn_session_msg::TurnOutcome::Succeeded,
        })
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Then nothing dispatches and nothing cancels — seed mode is inert.
    assert!(dispatched.is_empty(), "seed activation must not dispatch");
    assert!(canceled.is_empty(), "seed activation must not cancel");
}

#[rstest::rstest]
#[tokio::test]
async fn succeeded_parent_turn_fires_its_triggered_attendant() {
    // Given a parent with a reset-activated ParentCompleted attendant.
    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let dispatched = harness
        .spawn_recorder::<jinn_chat_input_msg::EnqueueUserMessage>()
        .await;
    let state = State::new(AppState::default());
    {
        let mut s = state.write();
        let parent = ChatSessionState::new();
        let parent_id = parent.session_id().clone();
        s.session.insert(parent);
        let mut attendant = {
            let read = s.session.get(&parent_id).expect("parent").clone();
            ChatSessionState::new_attendant(&read, true)
        };
        attendant.set_attendant_trigger(jinn_attendant_msg::AttendantTrigger::ParentCompleted);
        attendant.set_attendant_activation(jinn_attendant_msg::AttendantActivation::Reset);
        attendant.set_seed_template("verify: <prior report>".to_owned());
        attendant.append_attendant_report("prior finding".to_owned());
        s.session.insert(attendant);
    }
    let _actor = crate::trigger_actor::AttendantTriggerActor::spawn(
        harness.system(),
        crate::trigger_actor::AttendantTriggerActorDeps {
            services: harness.services().await,
            state: state.clone(),
        },
    );
    let parent_id = {
        let s = state.read();
        s.session
            .iter()
            .find(|(_, sess)| !sess.is_attendant())
            .map(|(id, _)| id.clone())
            .expect("parent")
    };

    // When the parent's turn completes successfully.
    harness
        .publish(jinn_session_msg::TurnCompleted {
            session_id: parent_id,
            outcome: jinn_session_msg::TurnOutcome::Succeeded,
        })
        .await;
    let dispatches =
        jinn_testutil::bus_harness::await_recorded::<jinn_chat_input_msg::EnqueueUserMessage>(
            &dispatched,
            1,
            std::time::Duration::from_secs(2),
        )
        .await;

    // Then exactly one dispatch went to the attendant, seeded through the
    // template with the prior report.
    assert_eq!(dispatches.len(), 1);
    let jinn_core_types::chat_entry::ChatEntryKind::User { display, .. } =
        &dispatches[0].entry.kind
    else {
        panic!("seed must be a user entry");
    };
    assert_eq!(display, "verify: prior finding");
    // And the attendant's turn is marked automated.
    let s = state.read();
    let attendant = s
        .session
        .iter()
        .find(|(_, sess)| sess.is_attendant())
        .map(|(_, sess)| sess)
        .expect("attendant");
    assert!(attendant.is_turn_automated());
}

#[rstest::rstest]
#[tokio::test]
async fn errored_and_canceled_turns_fire_nothing() {
    // Given a parent with a reset-activated ParentCompleted attendant.
    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let dispatched = harness
        .spawn_recorder::<jinn_chat_input_msg::EnqueueUserMessage>()
        .await;
    let state = State::new(AppState::default());
    {
        let mut s = state.write();
        let parent = ChatSessionState::new();
        let parent_id = parent.session_id().clone();
        s.session.insert(parent);
        let mut attendant = {
            let read = s.session.get(&parent_id).expect("parent").clone();
            ChatSessionState::new_attendant(&read, true)
        };
        attendant.set_attendant_trigger(jinn_attendant_msg::AttendantTrigger::ParentCompleted);
        attendant.set_attendant_activation(jinn_attendant_msg::AttendantActivation::Reset);
        s.session.insert(attendant);
    }
    let _actor = crate::trigger_actor::AttendantTriggerActor::spawn(
        harness.system(),
        crate::trigger_actor::AttendantTriggerActorDeps {
            services: harness.services().await,
            state: state.clone(),
        },
    );
    let parent_id = {
        let s = state.read();
        s.session
            .iter()
            .find(|(_, sess)| !sess.is_attendant())
            .map(|(id, _)| id.clone())
            .expect("parent")
    };

    // When an errored completion is published, then a cancelled one.
    harness
        .publish(jinn_session_msg::TurnCompleted {
            session_id: parent_id.clone(),
            outcome: jinn_session_msg::TurnOutcome::Error,
        })
        .await;
    harness
        .publish(jinn_session_msg::TurnCompleted {
            session_id: parent_id,
            outcome: jinn_session_msg::TurnOutcome::Canceled,
        })
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Then nothing dispatches — only success is worth verifying against.
    assert!(
        dispatched.is_empty(),
        "an errored or cancelled turn must not fire attendants"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn manual_trigger_attendant_does_not_fire_on_parent_completion() {
    // Given a parent with a reset-activated but Manual-trigger attendant.
    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let dispatched = harness
        .spawn_recorder::<jinn_chat_input_msg::EnqueueUserMessage>()
        .await;
    let state = State::new(AppState::default());
    {
        let mut s = state.write();
        let parent = ChatSessionState::new();
        let parent_id = parent.session_id().clone();
        s.session.insert(parent);
        let mut attendant = {
            let read = s.session.get(&parent_id).expect("parent").clone();
            ChatSessionState::new_attendant(&read, true)
        };
        // Trigger stays Manual.
        attendant.set_attendant_activation(jinn_attendant_msg::AttendantActivation::Reset);
        s.session.insert(attendant);
    }
    let _actor = crate::trigger_actor::AttendantTriggerActor::spawn(
        harness.system(),
        crate::trigger_actor::AttendantTriggerActorDeps {
            services: harness.services().await,
            state: state.clone(),
        },
    );
    let parent_id = {
        let s = state.read();
        s.session
            .iter()
            .find(|(_, sess)| !sess.is_attendant())
            .map(|(id, _)| id.clone())
            .expect("parent")
    };

    // When the parent's turn completes successfully.
    harness
        .publish(jinn_session_msg::TurnCompleted {
            session_id: parent_id,
            outcome: jinn_session_msg::TurnOutcome::Succeeded,
        })
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Then nothing dispatches — the attendant only runs when asked.
    assert!(dispatched.is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn a_fired_attendant_turn_is_marked_automated() {
    // Given a parent with a reset-activated ParentCompleted attendant.
    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let dispatched = harness
        .spawn_recorder::<jinn_chat_input_msg::EnqueueUserMessage>()
        .await;
    let state = State::new(AppState::default());
    let parent_id = {
        let mut s = state.write();
        let parent = ChatSessionState::new();
        let parent_id = parent.session_id().clone();
        s.session.insert(parent);
        let mut attendant = {
            let read = s.session.get(&parent_id).expect("parent").clone();
            ChatSessionState::new_attendant(&read, true)
        };
        attendant.set_attendant_trigger(jinn_attendant_msg::AttendantTrigger::ParentCompleted);
        attendant.set_attendant_activation(jinn_attendant_msg::AttendantActivation::Reset);
        s.session.insert(attendant);
        parent_id
    };
    let _actor = crate::trigger_actor::AttendantTriggerActor::spawn(
        harness.system(),
        crate::trigger_actor::AttendantTriggerActorDeps {
            services: harness.services().await,
            state: state.clone(),
        },
    );

    // When the parent's turn completes successfully and the attendant fires.
    harness
        .publish(jinn_session_msg::TurnCompleted {
            session_id: parent_id,
            outcome: jinn_session_msg::TurnOutcome::Succeeded,
        })
        .await;
    jinn_testutil::bus_harness::await_recorded::<jinn_chat_input_msg::EnqueueUserMessage>(
        &dispatched,
        1,
        std::time::Duration::from_secs(2),
    )
    .await;

    // Then the attendant's own turn is marked as automation-started — the
    // marker the publisher consumes so the exchange cannot loop unattended.
    let s = state.read();
    let attendant = s
        .session
        .iter()
        .find(|(_, sess)| sess.is_attendant())
        .map(|(_, sess)| sess)
        .expect("attendant");
    assert!(
        attendant.is_turn_automated(),
        "a fired attendant's turn must be marked automated"
    );
}
