#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::match_wildcard_for_single_variants,
    reason = "test code"
)]
use super::actor_channel::ActorChannelService;
use crate::common::core::app_msg::AppMsg;

#[rstest::rstest]
fn send_command_delivers_on_channel() {
    // Given an ActorChannelService with a sender/receiver pair.
    let (tx, rx) = kanal::unbounded::<AppMsg>();
    let svc = ActorChannelService::new(tx);

    // When sending a command.
    svc.send_command(crate::protocol::Command::RefreshModels);

    // Then the message arrives on the channel.
    let msg: AppMsg = rx.try_recv().expect("should receive").expect("non-none");
    match msg {
        AppMsg::Command { command, source } => {
            assert!(matches!(command, crate::protocol::Command::RefreshModels));
            assert!(source.is_none());
        }
        other => panic!("expected Command, got {other:?}"),
    }
}

#[rstest::rstest]
fn send_event_delivers_on_channel() {
    // Given an ActorChannelService with a sender/receiver pair.
    let (tx, rx) = kanal::unbounded::<AppMsg>();
    let svc = ActorChannelService::new(tx);

    // When sending an event.
    svc.send_event(crate::protocol::Event::ActorStarted(
        crate::common::actor::protocol::event::ActorStarted {
            name: "test".into(),
            description: None,
        },
    ));

    // Then the message arrives on the channel.
    let msg: AppMsg = rx.try_recv().expect("should receive").expect("non-none");
    match msg {
        AppMsg::Event { event, source } => {
            assert!(matches!(event, crate::protocol::Event::ActorStarted(_)));
            assert!(source.is_none());
        }
        other => panic!("expected Event, got {other:?}"),
    }
}

#[rstest::rstest]
fn send_delivers_raw_message() {
    // Given an ActorChannelService with a sender/receiver pair.
    let (tx, rx) = kanal::unbounded::<AppMsg>();
    let svc = ActorChannelService::new(tx);

    // When sending a raw AppMsg.
    svc.send(AppMsg::Command {
        command: crate::protocol::Command::RefreshModels,
        source: Some(crate::common::actor::actor_name::ActorName::new(
            "test-source",
        )),
    });

    // Then the raw message arrives on the channel.
    let msg: AppMsg = rx.try_recv().expect("should receive").expect("non-none");
    match msg {
        AppMsg::Command { command, source } => {
            assert!(matches!(command, crate::protocol::Command::RefreshModels));
            assert_eq!(source.as_deref(), Some("test-source"));
        }
        other => panic!("expected Command, got {other:?}"),
    }
}
