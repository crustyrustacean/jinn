#![allow(clippy::expect_used, clippy::indexing_slicing)]

use super::context::*;
use crate::ActorRef;
use crate::common::actor::envelope::ActorEnvelope;
use crate::common::actor::message_sink::MessageSink;
use crate::protocol::{Command, Event};
use std::sync::Arc;

fn test_sink() -> Arc<dyn MessageSink> {
    Arc::new(crate::common::actor::message_sink::RecordingSink::new())
}

fn test_sink_as_concrete() -> Arc<crate::common::actor::message_sink::RecordingSink> {
    Arc::new(crate::common::actor::message_sink::RecordingSink::new())
}

#[rstest::rstest]
fn subscribe_event_accumulates() {
    // Given a new context.
    let mut ctx = ActorContext::new("test", test_sink());

    // When subscribing to two events.
    ctx.subscribe_event_by_name("system::KeyDown");
    ctx.subscribe_event_by_name("chat_input::ChatEntrySubmitted");

    // Then take_registrations returns both subscriptions.
    let (subscriptions, _, _) = ctx.take_registrations();
    assert_eq!(
        subscriptions,
        vec!["system::KeyDown", "chat_input::ChatEntrySubmitted"]
    );
}

#[rstest::rstest]
fn subscribe_command_accumulates() {
    // Given a new context.
    let mut ctx = ActorContext::new("test", test_sink());

    // When subscribing to two commands.
    ctx.subscribe_command_by_name("echo");
    ctx.subscribe_command_by_name("reverse");

    // Then take_registrations returns both commands.
    let (_, commands, _) = ctx.take_registrations();
    assert_eq!(commands, vec!["echo", "reverse"]);
}

#[rstest::rstest]
fn first_take_returns_data() {
    // Given a context with registrations.
    let mut ctx = ActorContext::new("test", test_sink());
    ctx.subscribe_command_by_name("echo");
    ctx.subscribe_event_by_name("system::KeyDown");

    // When taking registrations.
    let first = ctx.take_registrations();

    // Then first has data.
    assert!(!first.0.is_empty());
    assert!(!first.1.is_empty());
}

#[rstest::rstest]
fn second_take_returns_empty() {
    // Given a context with registrations.
    let mut ctx = ActorContext::new("test", test_sink());
    ctx.subscribe_command_by_name("echo");
    ctx.subscribe_event_by_name("system::KeyDown");

    // When taking registrations twice.
    let _first = ctx.take_registrations();
    let second = ctx.take_registrations();

    // Then second is empty.
    assert!(second.0.is_empty());
    assert!(second.1.is_empty());
}

#[rstest::rstest]
fn set_and_take_actor_ref() {
    // Given a context with an ActorRef<String> stored.
    let mut ctx = ActorContext::new("test", test_sink());

    let (tx_actor, _) = kanal::unbounded::<ActorEnvelope<String>>();
    let actor_ref = ActorRef::new(tx_actor);
    ctx.set_actor_ref(actor_ref);

    // When taking the ActorRef<String>.
    let result = ctx.take_actor_ref::<String>();

    // Then it is Some.
    assert!(result.is_some());
}

#[rstest::rstest]
fn take_actor_ref_returns_none_when_empty() {
    // Given a context with no actor refs.
    let mut ctx = ActorContext::new("test", test_sink());

    // When taking an ActorRef<String>.
    let result = ctx.take_actor_ref::<String>();

    // Then it is None.
    assert!(result.is_none());
}

#[rstest::rstest]
fn take_actor_ref_removes_from_context() {
    // Given a context with an ActorRef<String> stored.
    let mut ctx = ActorContext::new("test", test_sink());

    let (tx_actor, _) = kanal::unbounded::<ActorEnvelope<String>>();
    ctx.set_actor_ref(ActorRef::new(tx_actor));

    // When taking it twice.
    let first = ctx.take_actor_ref::<String>();
    let second = ctx.take_actor_ref::<String>();

    // Then first is Some and second is None.
    assert!(first.is_some());
    assert!(second.is_none());
}

#[rstest::rstest]
fn send_command_delegates_to_sink() {
    // Given a context with a test sink.
    let sink = test_sink_as_concrete();
    let ctx = ActorContext::new("test", sink.clone());

    // When sending a command.
    ctx.send_command(Command::RefreshModels)
        .expect("send should succeed");

    // Then the sink recorded the command.
    let commands = sink.commands();
    assert_eq!(commands.len(), 1);
    assert!(matches!(commands[0], Command::RefreshModels));
}

#[rstest::rstest]
fn send_event_delegates_to_sink() {
    // Given a context with a test sink.
    let sink = test_sink_as_concrete();
    let ctx = ActorContext::new("test", sink.clone());

    // When sending a KeyDown event.
    ctx.send_event(Event::KeyDown(crate::protocol::system::KeyDown {
        key: crate::protocol::KeyEvent {
            key: crate::protocol::Key::Enter,
            modifiers: crate::protocol::Modifiers::none(),
        },
    }))
    .expect("send should succeed");

    // Then the sink recorded the event.
    let events = sink.events();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], Event::KeyDown(..)));
}

#[rstest::rstest]
fn name_returns_host_assigned_name() {
    // Given a context with a name.
    let ctx = ActorContext::new("my-actor", test_sink());

    // When querying the context name.
    assert_eq!(ctx.name(), "my-actor");

    // Then name returns the assigned name.
}

#[rstest::rstest]
fn sink_returns_arc_clone() {
    // Given a context with a test sink.
    let sink = test_sink_as_concrete();
    let ctx = ActorContext::new("test", sink.clone());

    // When getting the sink accessor.
    let cloned = ctx.sink();

    // Then it can send commands that the original sink records.
    cloned.send_command(Command::RefreshModels).expect("send");
    assert_eq!(sink.commands().len(), 1);
}

#[rstest::rstest]
fn set_description_stores_description() {
    // Given a new context.
    let mut ctx = ActorContext::new("test", test_sink());

    // When setting a description.
    ctx.set_description("does something useful");

    // Then description() returns the value.
    assert_eq!(ctx.description(), Some("does something useful"));
}

#[rstest::rstest]
fn description_is_none_by_default() {
    // Given a new context.
    let ctx = ActorContext::new("test", test_sink());

    // When querying description without setting it.
    assert!(ctx.description().is_none());

    // Then it is None.
}

#[rstest::rstest]
fn announce_started_includes_description() {
    // Given a context with a description.
    let sink = test_sink_as_concrete();
    let mut ctx = ActorContext::new("my-actor", sink.clone());
    ctx.set_description("does cool stuff");

    // When announcing started.
    ctx.announce_started();

    // Then the event carries the description.
    let events = sink.events();
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::ActorStarted(payload) => {
            assert_eq!(payload.name, "my-actor");
            assert_eq!(payload.description.as_deref(), Some("does cool stuff"));
        }
        other => panic!("expected ActorStarted, got {other:?}"),
    }
}

#[rstest::rstest]
fn announce_started_sends_actor_started_event() {
    // Given a context with a test sink.
    let sink = test_sink_as_concrete();
    let ctx = ActorContext::new("my-actor", sink.clone());

    // When announcing started.
    ctx.announce_started();

    // Then the sink recorded an ActorStarted event with the actor's name.
    let events = sink.events();
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::ActorStarted(payload) => {
            assert_eq!(payload.name, "my-actor");
            assert!(payload.description.is_none());
        }
        other => panic!("expected ActorStarted, got {other:?}"),
    }
}

#[rstest::rstest]
fn announce_shutdown_completed_sends_actor_shutdown_completed_event() {
    // Given a context with a test sink.
    let sink = test_sink_as_concrete();
    let ctx = ActorContext::new("my-actor", sink.clone());

    // When announcing shutdown completed.
    ctx.announce_shutdown_completed();

    // Then the sink recorded an ActorShutdownCompleted event with the actor's name.
    let events = sink.events();
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::ActorShutdownCompleted(payload) => {
            assert_eq!(payload.name, "my-actor");
        }
        other => panic!("expected ActorShutdownCompleted, got {other:?}"),
    }
}
