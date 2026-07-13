#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::time::Duration;

use super::*;
use crate::common::app_state::AppState;
use crate::common::bus::test_harness::{Recorder, TestHarness, await_recorded};
use crate::protocol::ChatEntry;

fn test_state() -> State {
    State::new(AppState::default())
}

fn test_counter() -> TiktokenCounter {
    TiktokenCounter::o200k_base()
}

struct QueueTestHarness {
    harness: TestHarness,
    send_recorder: kameo::actor::ActorRef<Recorder<SendToLlmProvider>>,
    chat_recorder: kameo::actor::ActorRef<Recorder<ChatEntrySubmitted>>,
    persist_recorder: kameo::actor::ActorRef<Recorder<PersistSession>>,
    state: State,
}

impl QueueTestHarness {
    async fn new() -> Self {
        let harness = TestHarness::new().await;
        let state = test_state();
        let counter = test_counter();

        let _queue = harness
            .spawn_actor::<super::QueueActor>(QueueActorDeps {
                state: state.clone(),
                counter,
                deps: harness.actor_deps().await,
            })
            .await;

        let send_recorder = harness.spawn_recorder::<SendToLlmProvider>().await;
        let chat_recorder = harness.spawn_recorder::<ChatEntrySubmitted>().await;
        let persist_recorder = harness.spawn_recorder::<PersistSession>().await;

        Self {
            harness,
            send_recorder,
            chat_recorder,
            persist_recorder,
            state,
        }
    }

    async fn publish<M: Clone + Send + 'static>(&self, msg: M) {
        self.harness.publish(msg).await;
    }
}

#[tokio::test]
async fn session_phase_changed_idle_pops_user_message_from_queue() {
    // Given a session with a queued user message in Idle phase.
    let th = QueueTestHarness::new().await;
    let session_id = {
        let mut s = th.state.write_test();
        let session = s.active_session_mut();
        session.enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user(
            "queued message",
        ))));
        s.session.active_session_id().clone()
    };

    // When publishing a SessionPhaseChanged with Idle phase.
    th.publish(SessionPhaseChanged {
        session_id: session_id.clone(),
        old_phase: PhaseKind::Sending,
        new_phase: PhaseKind::Idle,
    })
    .await;

    // Then SendToLlmProvider was emitted for the queued message.
    let send_cmds = await_recorded(&th.send_recorder, 1, Duration::from_millis(500)).await;
    assert!(
        !send_cmds.is_empty(),
        "expected SendToLlmProvider command for queued user message"
    );

    // And the queue is empty.
    let s = th.state.read();
    let session = s.session.get(&session_id).expect("session exists");
    assert!(
        session.queue_len() == 0,
        "expected queue to be empty after dispatch"
    );
}

#[tokio::test]
async fn session_phase_changed_non_idle_does_not_pop_queue() {
    // Given a session with a queued user message in non-Idle phase.
    let th = QueueTestHarness::new().await;
    let session_id = {
        let mut s = th.state.write_test();
        let session = s.active_session_mut();
        session.enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user(
            "queued message",
        ))));
        session.begin_sending();
        s.session.active_session_id().clone()
    };

    // When publishing a SessionPhaseChanged with Sending phase.
    th.publish(SessionPhaseChanged {
        session_id: session_id.clone(),
        old_phase: PhaseKind::Streaming,
        new_phase: PhaseKind::Sending,
    })
    .await;

    // Then no SendToLlmProvider was emitted.
    let send_cmds = await_recorded(&th.send_recorder, 1, Duration::from_millis(500)).await;
    assert!(
        send_cmds.is_empty(),
        "expected no SendToLlmProvider for non-Idle phase"
    );

    // And the queue still has the item.
    let s = th.state.read();
    let session = s.session.get(&session_id).expect("session exists");
    assert_eq!(session.queue_len(), 1);
}

#[tokio::test]
async fn session_phase_changed_idle_with_empty_queue_is_noop() {
    // Given a session with an empty queue.
    let th = QueueTestHarness::new().await;
    let session_id = {
        let s = th.state.read();
        s.session.active_session_id().clone()
    };

    // When publishing a SessionPhaseChanged with Idle phase.
    th.publish(SessionPhaseChanged {
        session_id: session_id.clone(),
        old_phase: PhaseKind::Streaming,
        new_phase: PhaseKind::Idle,
    })
    .await;

    // Then no commands were emitted.
    let send_cmds = await_recorded(&th.send_recorder, 1, Duration::from_millis(500)).await;
    assert!(send_cmds.is_empty(), "expected no commands for empty queue");
}

#[tokio::test]
async fn dispatch_user_message_emits_chat_entry_submitted() {
    // Given a queue actor wired to the bus.
    let th = QueueTestHarness::new().await;
    let session_id = {
        let s = th.state.read();
        s.session.active_session_id().clone()
    };

    // When dispatching a user message by publishing an Idle transition.
    {
        let mut s = th.state.write_test();
        s.active_session_mut()
            .enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user("hello"))));
    }
    th.publish(SessionPhaseChanged {
        session_id: session_id.clone(),
        old_phase: PhaseKind::Sending,
        new_phase: PhaseKind::Idle,
    })
    .await;

    // Then ChatEntrySubmitted was emitted.
    let chat_events = await_recorded(&th.chat_recorder, 1, Duration::from_millis(500)).await;
    assert!(!chat_events.is_empty(), "expected ChatEntrySubmitted event");
}

#[tokio::test]
async fn dispatch_user_message_sets_title_on_first_message() {
    // Given a session with no title.
    let th = QueueTestHarness::new().await;
    let session_id = {
        let s = th.state.read();
        s.session.active_session_id().clone()
    };

    // When dispatching a user message.
    {
        let mut s = th.state.write_test();
        s.active_session_mut()
            .enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user(
                "my new chat",
            ))));
    }
    th.publish(SessionPhaseChanged {
        session_id: session_id.clone(),
        old_phase: PhaseKind::Sending,
        new_phase: PhaseKind::Idle,
    })
    .await;

    // Then the session title was set to the first line of the message.
    let _ = await_recorded(&th.send_recorder, 1, Duration::from_millis(500)).await;
    let s = th.state.read();
    let session = s.session.get(&session_id).expect("session exists");
    assert_eq!(session.title(), Some("my new chat"));
}

#[tokio::test]
async fn dispatch_user_message_transitions_to_sending() {
    // Given a session in Idle phase.
    let th = QueueTestHarness::new().await;
    let session_id = {
        let s = th.state.read();
        s.session.active_session_id().clone()
    };

    // When dispatching a user message.
    {
        let mut s = th.state.write_test();
        s.active_session_mut()
            .enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user("hello"))));
    }
    th.publish(SessionPhaseChanged {
        session_id: session_id.clone(),
        old_phase: PhaseKind::Sending,
        new_phase: PhaseKind::Idle,
    })
    .await;

    // Then the session is in Sending phase.
    let _ = await_recorded(&th.send_recorder, 1, Duration::from_millis(500)).await;
    let s = th.state.read();
    let session = s.session.get(&session_id).expect("session exists");
    assert!(matches!(session.phase(), PhaseKind::Sending));
}

#[tokio::test]
async fn dispatch_tool_continuation_emits_send_to_llm_provider() {
    // Given a session in Idle phase with history and a tool continuation queued.
    let th = QueueTestHarness::new().await;
    let session_id = {
        let mut s = th.state.write_test();
        let session = s.active_session_mut();
        session.push_entry(ChatEntry::user("previous message"));
        session.enqueue(QueueItem::ToolContinuation);
        s.session.active_session_id().clone()
    };

    // When dispatching a tool continuation via Idle transition.
    th.publish(SessionPhaseChanged {
        session_id: session_id.clone(),
        old_phase: PhaseKind::Sending,
        new_phase: PhaseKind::Idle,
    })
    .await;

    // Then SendToLlmProvider was emitted.
    let send_cmds = await_recorded(&th.send_recorder, 1, Duration::from_millis(500)).await;
    assert!(!send_cmds.is_empty(), "expected SendToLlmProvider command");
}

#[tokio::test]
async fn dispatch_user_message_provider_id_is_none_when_no_provider() {
    // Given a session with the default model (NO_PROVIDER_ID).
    let th = QueueTestHarness::new().await;
    let session_id = {
        let s = th.state.read();
        s.session.active_session_id().clone()
    };

    // When dispatching a user message.
    {
        let mut s = th.state.write_test();
        s.active_session_mut()
            .enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user("hello"))));
    }
    th.publish(SessionPhaseChanged {
        session_id: session_id.clone(),
        old_phase: PhaseKind::Sending,
        new_phase: PhaseKind::Idle,
    })
    .await;

    // Then SendToLlmProvider has provider_id = None.
    let send_cmds = await_recorded(&th.send_recorder, 1, Duration::from_millis(500)).await;
    let provider_id = send_cmds.first().map(|c| c.provider_id.clone());
    assert_eq!(
        provider_id,
        Some(None),
        "expected provider_id None for NO_PROVIDER_ID"
    );
}

#[tokio::test]
async fn dispatch_user_message_provider_id_is_some_when_model_set() {
    // Given a session with an explicit model.
    let th = QueueTestHarness::new().await;
    let session_id = {
        let mut s = th.state.write_test();
        s.active_session_mut().set_model("my-model".to_owned());
        s.active_session_mut()
            .enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user("hello"))));
        s.session.active_session_id().clone()
    };

    // When dispatching a user message.
    th.publish(SessionPhaseChanged {
        session_id: session_id.clone(),
        old_phase: PhaseKind::Sending,
        new_phase: PhaseKind::Idle,
    })
    .await;

    // Then SendToLlmProvider has provider_id = Some("my-model").
    let send_cmds = await_recorded(&th.send_recorder, 1, Duration::from_millis(500)).await;
    let provider_id = send_cmds.first().map(|c| c.provider_id.clone());
    assert_eq!(
        provider_id,
        Some(Some("my-model".to_owned())),
        "expected provider_id Some(\"my-model\")"
    );
}

#[tokio::test]
async fn dispatch_tool_continuation_provider_id_is_none_when_no_provider() {
    // Given a session with the default model (NO_PROVIDER_ID) and history.
    let th = QueueTestHarness::new().await;
    let session_id = {
        let mut s = th.state.write_test();
        let session = s.active_session_mut();
        session.push_entry(ChatEntry::user("previous message"));
        session.enqueue(QueueItem::ToolContinuation);
        s.session.active_session_id().clone()
    };

    // When dispatching a tool continuation.
    th.publish(SessionPhaseChanged {
        session_id: session_id.clone(),
        old_phase: PhaseKind::Sending,
        new_phase: PhaseKind::Idle,
    })
    .await;

    // Then SendToLlmProvider has provider_id = None.
    let send_cmds = await_recorded(&th.send_recorder, 1, Duration::from_millis(500)).await;
    let provider_id = send_cmds.first().map(|c| c.provider_id.clone());
    assert_eq!(
        provider_id,
        Some(None),
        "expected provider_id None for NO_PROVIDER_ID in tool continuation"
    );
}

#[tokio::test]
async fn dispatch_tool_continuation_provider_id_is_some_when_model_set() {
    // Given a session with an explicit model and history.
    let th = QueueTestHarness::new().await;
    let session_id = {
        let mut s = th.state.write_test();
        let session = s.active_session_mut();
        session.push_entry(ChatEntry::user("previous message"));
        session.enqueue(QueueItem::ToolContinuation);
        s.active_session_mut().set_model("tool-model".to_owned());
        s.session.active_session_id().clone()
    };

    // When dispatching a tool continuation.
    th.publish(SessionPhaseChanged {
        session_id: session_id.clone(),
        old_phase: PhaseKind::Sending,
        new_phase: PhaseKind::Idle,
    })
    .await;

    // Then SendToLlmProvider has provider_id = Some("tool-model").
    let send_cmds = await_recorded(&th.send_recorder, 1, Duration::from_millis(500)).await;
    let provider_id = send_cmds.first().map(|c| c.provider_id.clone());
    assert_eq!(
        provider_id,
        Some(Some("tool-model".to_owned())),
        "expected provider_id Some(\"tool-model\") in tool continuation"
    );
}

#[tokio::test]
async fn dispatch_user_message_drains_steering_buffer_before_assembly() {
    // Given a session with a non-empty steering buffer and a queued user message.
    let th = QueueTestHarness::new().await;
    let session_id = {
        let mut s = th.state.write_test();
        let session = s.active_session_mut();
        session
            .steering_buffer_mut()
            .push_fragment("steer here".to_owned());
        session.enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user("hello"))));
        s.session.active_session_id().clone()
    };

    // When dispatching a user message via Idle transition.
    th.publish(SessionPhaseChanged {
        session_id: session_id.clone(),
        old_phase: PhaseKind::Sending,
        new_phase: PhaseKind::Idle,
    })
    .await;

    // Then the steering buffer is drained before assembly.
    let _ = await_recorded(&th.send_recorder, 1, Duration::from_millis(500)).await;
    let s = th.state.read();
    let session = s.session.get(&session_id).expect("session");
    assert!(
        session.steering_buffer().is_empty(),
        "steering buffer must be drained during dispatch_user_message"
    );

    // And the drained steering entry appears in history.
    let has_steering_entry = session.history().iter().any(|e| {
        matches!(
            &e.kind,
            crate::protocol::ChatEntryKind::User { expanded, .. } if expanded == "steer here"
        )
    });
    assert!(
        has_steering_entry,
        "drained steering entry must appear in history after dispatch_user_message"
    );
}

#[tokio::test]
async fn dispatch_resume_drains_steering_buffer_before_assembly() {
    // Given a session with a non-empty steering buffer and a tool continuation queued.
    let th = QueueTestHarness::new().await;
    let session_id = {
        let mut s = th.state.write_test();
        let session = s.active_session_mut();
        session.push_entry(ChatEntry::user("previous"));
        session
            .steering_buffer_mut()
            .push_fragment("resume steer".to_owned());
        session.enqueue(QueueItem::ToolContinuation);
        s.session.active_session_id().clone()
    };

    // When dispatching a resume via Idle transition.
    th.publish(SessionPhaseChanged {
        session_id: session_id.clone(),
        old_phase: PhaseKind::Sending,
        new_phase: PhaseKind::Idle,
    })
    .await;

    // Then the steering buffer is drained before assembly.
    let _ = await_recorded(&th.send_recorder, 1, Duration::from_millis(500)).await;
    let s = th.state.read();
    let session = s.session.get(&session_id).expect("session");
    assert!(
        session.steering_buffer().is_empty(),
        "steering buffer must be drained during dispatch_resume"
    );

    // And the drained steering entry appears in history.
    let has_steering_entry = session.history().iter().any(|e| {
        matches!(
            &e.kind,
            crate::protocol::ChatEntryKind::User { expanded, .. } if expanded == "resume steer"
        )
    });
    assert!(
        has_steering_entry,
        "drained steering entry must appear in history after dispatch_resume"
    );
}
