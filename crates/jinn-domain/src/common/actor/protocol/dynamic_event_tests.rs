//! Tests for DynamicEvent.

use serde_json::json;

use crate::common::actor::event_msg::EventMsg;
use crate::common::actor::protocol::dynamic_event::DynamicEvent;

#[rstest::rstest]
fn dynamic_event_serializes_round_trip() {
    // Given a DynamicEvent with a string name and JSON object payload.
    let evt = DynamicEvent {
        name: "test::event".to_owned(),
        payload: json!({"key": "value"}),
    };

    // When serializing and deserializing.
    let json = serde_json::to_string(&evt).expect("serialize");
    let back: DynamicEvent = serde_json::from_str(&json).expect("deserialize");

    // Then all fields are preserved.
    assert_eq!(back.name, "test::event");
    assert_eq!(back.payload["key"], "value");
}

#[rstest::rstest]
fn dynamic_event_carries_nested_json() {
    // Given a DynamicEvent with deeply nested JSON.
    let evt = DynamicEvent {
        name: "nested::event".to_owned(),
        payload: json!({
            "nested": {
                "deep": [1, 2, 3]
            }
        }),
    };

    // When serializing and deserializing.
    let json = serde_json::to_string(&evt).expect("serialize");
    let back: DynamicEvent = serde_json::from_str(&json).expect("deserialize");

    // Then nested structure is preserved.
    assert_eq!(back.payload["nested"]["deep"][2], 3);
}

#[rstest::rstest]
fn dynamic_event_msg_type_name_is_dynamic() {
    // Given a DynamicEvent.
    // Then its EventMsg::TYPE_NAME constant is "dynamic".
    assert_eq!(DynamicEvent::TYPE_NAME, "dynamic");
}
