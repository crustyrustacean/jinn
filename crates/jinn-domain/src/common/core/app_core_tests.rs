#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::match_wildcard_for_single_variants,
    reason = "test code"
)]

use super::app_core::*;
use crate::common::core::app_msg::AppMsg;

#[rstest::rstest]
#[tokio::test]
async fn submit_command_sends_on_channel() {
    // Given an AppCore with a sender/receiver pair.
    let (tx, rx) = kanal::unbounded::<AppMsg>();
    let state = crate::State::new(crate::common::app_state::AppState::default());
    let core = AppCore { state, sender: tx, bridge: crate::common::bridge::Bridge::new_for_test() };

    // When submitting a command.
    core.submit_command(crate::protocol::Command::RefreshModels);

    // Then the message was sent on the channel.
    let msg: AppMsg = rx
        .try_recv()
        .expect("should receive message")
        .expect("non-none");
    match msg {
        AppMsg::Command { command, source } => {
            assert!(matches!(command, crate::protocol::Command::RefreshModels));
            assert!(source.is_none());
        }
        other => panic!("expected Command msg, got {other:?}"),
    }
}
