#![allow(clippy::expect_used, clippy::indexing_slicing)]

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use crate::common::actor::{
    Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink, RecordingSink, SystemMessage,
};
use crate::common::actor_host::actor_host::ActorHost;
use crate::common::actor_host::in_memory::{
    ActorSpawnResult, InMemoryActorHost, ShutdownTracker, spawn_actor_impl,
};
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::protocol::{ActorName, Command, CommandMsg as _, Event};

/// No-op actor for lifecycle testing.
struct NoopActor;

impl Actor for NoopActor {
    type Message = String;
    type Deps = ();

    fn activate(_deps: Self::Deps, _ctx: &mut ActorContext) -> Self {
        Self
    }

    async fn handle(&mut self, _msg: ActorEnvelope<String>, _ctx: &ActorContext) {}

    async fn shutdown(self) {}
}

/// Actor that records received messages.
struct RecordingActor {
    received: Arc<Mutex<Vec<String>>>,
}

impl RecordingActor {
    fn new() -> (Self, Arc<parking_lot::Mutex<Vec<String>>>) {
        let received = Arc::new(Mutex::new(Vec::new()));
        let clone = received.clone();
        (Self { received }, clone)
    }
}

impl Actor for RecordingActor {
    type Message = String;
    type Deps = ();

    fn activate(_deps: Self::Deps, _ctx: &mut ActorContext) -> Self {
        panic!("use RecordingActor::new() and set subscriptions manually");
    }

    async fn handle(&mut self, msg: ActorEnvelope<String>, _ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Direct(s) => {
                self.received.lock().push(s);
            }
            ActorEnvelope::Event(e) => {
                self.received
                    .lock()
                    .push(format!("event:{}", e.type_name().unwrap_or("unknown")));
            }
            ActorEnvelope::Command(c) => {
                let name = format!("{c}");
                self.received.lock().push(format!("command:{name}"));
            }
            ActorEnvelope::System(msg) => {
                self.received.lock().push(format!("system:{msg:?}"));
            }
        }
    }

    async fn shutdown(self) {}
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("create runtime")
}

fn test_tracker() -> ShutdownTracker {
    ShutdownTracker::new()
}

fn spawn_noop_actor(
    name: &str,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
    tracker: &ShutdownTracker,
) -> ActorSpawnResult {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<String>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new(name, sink);
    let actor = NoopActor::activate((), &mut ctx);
    spawn_actor_impl(name, actor, &actor_ref, rx, ctx, handle, tracker.clone())
}

fn spawn_recording_actor(
    name: &str,
    sink: Arc<dyn MessageSink>,
    subscriptions: &[&str],
    commands: &[&str],
    handle: &tokio::runtime::Handle,
    tracker: &ShutdownTracker,
) -> (ActorSpawnResult, Arc<parking_lot::Mutex<Vec<String>>>) {
    let (actor, received) = RecordingActor::new();
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<String>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new(name, sink);
    for sub in subscriptions {
        ctx.subscribe_event_by_name(*sub);
    }
    for cmd in commands {
        ctx.subscribe_command_by_name(*cmd);
    }
    let result = spawn_actor_impl(name, actor, &actor_ref, rx, ctx, handle, tracker.clone());
    (result, received)
}

#[rstest::rstest]
fn host_routes_subscribed_event() {
    // Given a host with a recording actor subscribed to ChatEntrySubmitted.
    let runtime = rt();
    let tracker = test_tracker();
    let _guard = runtime.enter();
    let sink = Arc::new(RecordingSink::new());
    let (result, received) = spawn_recording_actor(
        "recorder",
        sink.clone(),
        &["chat_input::ChatEntrySubmitted"],
        &[],
        runtime.handle(),
        &tracker,
    );
    let host = InMemoryActorHost::from_actors_with_handle(
        vec![result],
        runtime.handle().clone(),
        tracker.clone(),
    );

    // When sending a subscribed event.
    let event = Event::ChatEntrySubmitted(ChatEntrySubmitted {
        session_id: crate::protocol::SessionId::new(),
        entry: crate::protocol::ChatEntry::user("hello"),
    });
    host.send_event(&event, None);
    std::thread::sleep(Duration::from_millis(50));

    // Then the actor received the event.
    let msgs = received.lock().clone();
    assert!(
        !msgs.is_empty(),
        "actor should receive the subscribed event"
    );
    assert!(
        msgs.iter().any(|m| m.contains("event:")),
        "expected event message, got: {msgs:?}"
    );

    host.shutdown_with_timeout(Duration::from_millis(200))
        .expect("shutdown");
}

#[rstest::rstest]
fn system_message_delivered_to_all() {
    // Given two actors with different subscriptions.
    let runtime = rt();
    let tracker = test_tracker();
    let _guard = runtime.enter();
    let sink = Arc::new(RecordingSink::new());
    let (r1, _received1) = spawn_recording_actor(
        "actor-a",
        sink.clone(),
        &["chat_input::ChatEntrySubmitted"],
        &[],
        runtime.handle(),
        &tracker,
    );
    let (r2, _received2) = spawn_recording_actor(
        "actor-b",
        sink.clone(),
        &["system::KeyDown"],
        &[],
        runtime.handle(),
        &tracker,
    );
    let host = InMemoryActorHost::from_actors_with_handle(
        vec![r1, r2],
        runtime.handle().clone(),
        tracker.clone(),
    );

    // When sending SystemMessage::ApplicationShuttingDown.
    host.send_system(SystemMessage::ApplicationShuttingDown);
    std::thread::sleep(Duration::from_millis(50));

    // Then both actors exit their run loop (shutdown_with_timeout succeeds).
    host.shutdown_with_timeout(Duration::from_millis(200))
        .expect("shutdown");
}

#[rstest::rstest]
fn host_routes_registered_command() {
    // Given a host with a recording actor registered for PushChatEntry.
    let runtime = rt();
    let tracker = test_tracker();
    let _guard = runtime.enter();
    let sink = Arc::new(RecordingSink::new());
    let (result, received) = spawn_recording_actor(
        "recorder",
        sink.clone(),
        &[],
        &[crate::feat::chat_input::protocol::command::PushChatEntry::NAME],
        runtime.handle(),
        &tracker,
    );
    let host = InMemoryActorHost::from_actors_with_handle(
        vec![result],
        runtime.handle().clone(),
        tracker.clone(),
    );

    // When sending a registered command.
    host.send_command(
        &Command::PushChatEntry(crate::feat::chat_input::protocol::command::PushChatEntry {
            session_id: crate::protocol::SessionId::new(),
            entry: crate::protocol::ChatEntry::user("test"),
        }),
        None,
    );
    std::thread::sleep(Duration::from_millis(50));

    // Then the actor received the command.
    let msgs = received.lock().clone();
    assert!(
        !msgs.is_empty(),
        "actor should receive the registered command"
    );
    assert!(
        msgs.iter().any(|m| m.contains("command:")),
        "expected command message, got: {msgs:?}"
    );

    host.shutdown_with_timeout(Duration::from_millis(200))
        .expect("shutdown");
}

#[rstest::rstest]
fn host_skips_unregistered_command() {
    // Given a host with a recording actor registered for PushChatEntry only.
    let runtime = rt();
    let tracker = test_tracker();
    let _guard = runtime.enter();
    let sink = Arc::new(RecordingSink::new());
    let (result, received) = spawn_recording_actor(
        "recorder",
        sink.clone(),
        &[],
        &[crate::feat::chat_input::protocol::command::PushChatEntry::NAME],
        runtime.handle(),
        &tracker,
    );
    let host = InMemoryActorHost::from_actors_with_handle(
        vec![result],
        runtime.handle().clone(),
        tracker.clone(),
    );

    // When sending an unregistered command (RefreshModels is not subscribed by the actor).
    host.send_command(&Command::RefreshModels, None);
    std::thread::sleep(Duration::from_millis(50));

    // Then no messages were delivered to the actor.
    let msgs = received.lock().clone();
    assert!(
        msgs.is_empty(),
        "actor should not receive unregistered command: {msgs:?}"
    );

    host.shutdown_with_timeout(Duration::from_millis(200))
        .expect("shutdown");
}

#[rstest::rstest]
#[should_panic(expected = "is subscribed by multiple actors")]
fn duplicate_command_subscription_panics() {
    // Given two actors subscribing to the same command.
    let runtime = rt();
    let _guard = runtime.enter();
    let sink = Arc::new(RecordingSink::new());
    let cmd_name = crate::feat::chat_input::protocol::command::PushChatEntry::NAME;
    let tracker = test_tracker();
    let (r1, _received1) = spawn_recording_actor(
        "actor-a",
        sink.clone(),
        &[],
        &[cmd_name],
        runtime.handle(),
        &tracker,
    );
    let (r2, _received2) = spawn_recording_actor(
        "actor-b",
        sink.clone(),
        &[],
        &[cmd_name],
        runtime.handle(),
        &tracker,
    );

    // When building the host.
    // Then it panics because both actors subscribe to PushChatEntry.
    let _host = InMemoryActorHost::from_actors_with_handle(
        vec![r1, r2],
        runtime.handle().clone(),
        tracker.clone(),
    );
}

#[rstest::rstest]
fn host_shutdown_joins_tasks() {
    // Given a running host with two actors.
    let runtime = rt();
    let tracker = test_tracker();
    let _guard = runtime.enter();
    let sink = Arc::new(RecordingSink::new());
    let r1 = spawn_noop_actor("a", sink.clone(), runtime.handle(), &tracker);
    let r2 = spawn_noop_actor("b", sink.clone(), runtime.handle(), &tracker);
    let host = InMemoryActorHost::from_actors_with_handle(
        vec![r1, r2],
        runtime.handle().clone(),
        tracker.clone(),
    );

    // When shutdown is called.
    host.shutdown_with_timeout(Duration::from_millis(200))
        .expect("shutdown");

    // Then all tasks are drained.
    let lifecycle = host.lifecycle.lock();
    assert!(lifecycle.tasks.is_empty());
}

#[rstest::rstest]
fn source_filtering_skips_originating_actor() {
    // Given two actors subscribed to the same event.
    let runtime = rt();
    let tracker = test_tracker();
    let _guard = runtime.enter();
    let sink = Arc::new(RecordingSink::new());
    let (r1, received1) = spawn_recording_actor(
        "actor-a",
        sink.clone(),
        &["chat_input::ChatEntrySubmitted"],
        &[],
        runtime.handle(),
        &tracker,
    );
    let (r2, received2) = spawn_recording_actor(
        "actor-b",
        sink.clone(),
        &["chat_input::ChatEntrySubmitted"],
        &[],
        runtime.handle(),
        &tracker,
    );
    let host = InMemoryActorHost::from_actors_with_handle(
        vec![r1, r2],
        runtime.handle().clone(),
        tracker.clone(),
    );

    // When sending an event with source of actor-a.
    let event = Event::ChatEntrySubmitted(ChatEntrySubmitted {
        session_id: crate::protocol::SessionId::new(),
        entry: crate::protocol::ChatEntry::user("hello"),
    });
    host.send_event(&event, Some(&ActorName::new("actor-a")));
    std::thread::sleep(Duration::from_millis(50));

    // Then actor-b receives it but actor-a does not.
    let msgs_a = received1.lock().clone();
    let msgs_b = received2.lock().clone();
    assert!(
        msgs_a.is_empty(),
        "actor-a should not receive the event: {msgs_a:?}"
    );
    assert!(!msgs_b.is_empty(), "actor-b should receive the event");

    host.shutdown_with_timeout(Duration::from_millis(200))
        .expect("shutdown");
}

#[rstest::rstest]
fn system_shutdown_delivered_to_all() {
    // Given two actors with different subscriptions.
    let runtime = rt();
    let tracker = test_tracker();
    let _guard = runtime.enter();
    let sink = Arc::new(RecordingSink::new());
    let (r1, _received1) = spawn_recording_actor(
        "actor-a",
        sink.clone(),
        &["chat_input::ChatEntrySubmitted"],
        &[],
        runtime.handle(),
        &tracker,
    );
    let (r2, _received2) = spawn_recording_actor(
        "actor-b",
        sink.clone(),
        &["system::KeyDown"],
        &[],
        runtime.handle(),
        &tracker,
    );
    let host = InMemoryActorHost::from_actors_with_handle(
        vec![r1, r2],
        runtime.handle().clone(),
        tracker.clone(),
    );

    // When sending SystemMessage::ApplicationShuttingDown.
    host.send_system(SystemMessage::ApplicationShuttingDown);
    std::thread::sleep(Duration::from_millis(50));

    // Then both actors exit their run loop (shutdown_with_timeout succeeds).
    host.shutdown_with_timeout(Duration::from_millis(200))
        .expect("shutdown");
}

#[rstest::rstest]
fn actor_to_actor_direct_message() {
    // Define a minimal actor for direct message testing.
    struct DirectActor;
    impl Actor for DirectActor {
        type Message = String;
        type Deps = ();
        fn activate(_deps: Self::Deps, _ctx: &mut ActorContext) -> Self {
            Self
        }
        async fn handle(&mut self, _msg: ActorEnvelope<String>, _ctx: &ActorContext) {}
        async fn shutdown(self) {}
    }

    // Given two actors where actor-a holds actor-b's ActorRef.
    let runtime = rt();
    let tracker = test_tracker();
    let _guard = runtime.enter();
    let sink = Arc::new(RecordingSink::new());

    // Create actor-b first to get its ref.
    let (actor_b, received_b) = RecordingActor::new();
    let (tx_b, rx_b) = kanal::unbounded::<ActorEnvelope<String>>();
    let ref_b = ActorRef::new(tx_b);
    let ctx_b = ActorContext::new("actor-b", sink.clone());
    let result_b = spawn_actor_impl(
        "actor-b",
        actor_b,
        &ref_b,
        rx_b,
        ctx_b,
        runtime.handle(),
        tracker.clone(),
    );

    // Create actor-a with ref_b injected.
    let (tx_a, rx_a) = kanal::unbounded::<ActorEnvelope<String>>();
    let actor_ref_a = ActorRef::new(tx_a);
    let mut ctx_a = ActorContext::new("actor-a", sink.clone());
    ctx_a.set_actor_ref(ref_b.clone());
    let actor_a = DirectActor::activate((), &mut ctx_a);
    let result_a = spawn_actor_impl(
        "actor-a",
        actor_a,
        &actor_ref_a,
        rx_a,
        ctx_a,
        runtime.handle(),
        tracker.clone(),
    );

    let _host = InMemoryActorHost::from_actors_with_handle(
        vec![result_a, result_b],
        runtime.handle().clone(),
        tracker.clone(),
    );

    // When sending a direct message to actor-b.
    ref_b.send("hello from a".to_owned()).expect("send");
    std::thread::sleep(Duration::from_millis(50));

    // Then actor-b receives the direct message.
    let msgs_b = received_b.lock().clone();
    assert!(
        msgs_b.iter().any(|m| m.contains("hello from a")),
        "actor-b should receive direct message: {msgs_b:?}"
    );
}
