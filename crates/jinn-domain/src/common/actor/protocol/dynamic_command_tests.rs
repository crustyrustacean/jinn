//! Tests for DynamicCommand.
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::string_slice,
    reason = "test code"
)]

use serde_json::json;

use crate::common::actor::command_msg::CommandMsg;
use crate::common::actor::protocol::dynamic_command::DynamicCommand;

#[rstest::rstest]
fn dynamic_command_serializes_round_trip() {
    // Given a DynamicCommand with a string name and JSON object payload.
    let cmd = DynamicCommand {
        name: "test::cmd".to_owned(),
        payload: json!({"key": "value"}),
    };

    // When serializing and deserializing.
    let json = serde_json::to_string(&cmd).expect("serialize");
    let back: DynamicCommand = serde_json::from_str(&json).expect("deserialize");

    // Then all fields are preserved.
    assert_eq!(back.name, "test::cmd");
    assert_eq!(back.payload["key"], "value");
}

#[rstest::rstest]
fn dynamic_command_carries_nested_json() {
    // Given a DynamicCommand with deeply nested JSON.
    let cmd = DynamicCommand {
        name: "nested::test".to_owned(),
        payload: json!({
            "nested": {
                "deep": [1, 2, 3]
            }
        }),
    };

    // When serializing and deserializing.
    let json = serde_json::to_string(&cmd).expect("serialize");
    let back: DynamicCommand = serde_json::from_str(&json).expect("deserialize");

    // Then nested structure is preserved.
    assert_eq!(back.payload["nested"]["deep"][2], 3);
}

#[rstest::rstest]
fn dynamic_command_msg_name_is_dynamic() {
    // Given a DynamicCommand.
    // Then its CommandMsg::NAME constant is "dynamic".
    assert_eq!(DynamicCommand::NAME, "dynamic");
}
