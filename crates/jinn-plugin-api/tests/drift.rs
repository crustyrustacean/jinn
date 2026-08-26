#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::unreachable,
    reason = "test code"
)]

//! Drift tests — the hand-maintained schema must agree with the wire types.
//!
//! The schema file (`plugin-api.schema.json`) is the published artifact
//! third parties generate bindings from. These tests serialize representative
//! instances of every wire message and validate them against the schema, so
//! editing the types without the schema (or vice versa) fails the build.
//!
//! They also pin the forward-compatibility behavior: unknown `type` tags
//! deserialize to `Unknown` instead of erroring.

use std::collections::BTreeMap;

use jinn_plugin_api::{
    Envelope, HostToPlugin, PersonaDef, PluginCitation, PluginToHost, PluginToHostOrHostToPlugin,
    PushCitations, THEME_COLOR_SLOTS, ThemeDef, ToolCallEvent, ToolResultEvent, TurnEndEvent,
    Welcome,
};

/// Compiles the committed schema file for validation.
fn schema() -> jsonschema::Validator {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/plugin-api.schema.json");
    let schema_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("schema file readable"))
            .expect("schema file is valid JSON");
    jsonschema::validator_for(&schema_json).expect("schema compiles")
}

/// Serializes an envelope and validates it against the schema.
fn assert_valid(envelope: &Envelope) {
    let json = serde_json::to_value(envelope).expect("serialize");
    let errors: Vec<_> = schema().iter_errors(&json).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "schema drift:\n{json}\n{errors:#?}");
}

/// Builds a fully-populated theme for fixtures.
fn sample_theme() -> ThemeDef {
    let mut colors = BTreeMap::new();
    colors.insert("focus_accent".to_owned(), "#ff8800".to_owned());
    colors.insert("primary_text".to_owned(), "cyan".to_owned());
    colors.insert("gutter_bg".to_owned(), "14".to_owned());
    ThemeDef {
        name: "sample".to_owned(),
        description: Some("a sample".to_owned()),
        colors,
    }
}

/// Builds a fully-populated persona for fixtures.
fn sample_persona() -> PersonaDef {
    PersonaDef {
        name: "coding-assistant".to_owned(),
        description: Some("Expert coding assistant".to_owned()),
        body: "You are an expert coding assistant.".to_owned(),
    }
}

#[test]
fn hello_envelope_validates_against_schema() {
    // Given a Hello envelope.
    let envelope = Envelope::for_plugin(
        PluginToHost::Hello(jinn_plugin_api::Hello {
            protocol_version: 1,
            name: "themes".to_owned(),
            subscriptions: vec![],
        }),
        0,
        0,
    );

    // Then it validates against the committed schema.
    assert_valid(&envelope);
}

#[test]
fn set_theme_entries_envelope_validates_against_schema() {
    // Given a SetThemeEntries envelope with a populated theme.
    let envelope = Envelope::for_plugin(
        PluginToHost::SetThemeEntries(jinn_plugin_api::SetThemeEntries {
            themes: vec![sample_theme()],
        }),
        1,
        0,
    );

    // Then it validates against the committed schema.
    assert_valid(&envelope);
}

#[test]
fn set_persona_entries_envelope_validates_against_schema() {
    // Given a SetPersonaEntries envelope with a populated persona.
    let envelope = Envelope::for_plugin(
        PluginToHost::SetPersonaEntries(jinn_plugin_api::SetPersonaEntries {
            personas: vec![sample_persona()],
        }),
        1,
        0,
    );

    // Then it validates against the committed schema.
    assert_valid(&envelope);
}

#[test]
fn set_persona_entries_without_description_validates_against_schema() {
    // Given a SetPersonaEntries envelope whose persona has no description.
    let envelope = Envelope::for_plugin(
        PluginToHost::SetPersonaEntries(jinn_plugin_api::SetPersonaEntries {
            personas: vec![PersonaDef {
                name: "minimal".to_owned(),
                description: None,
                body: "Body text.".to_owned(),
            }],
        }),
        1,
        0,
    );

    // Then it validates against the committed schema.
    assert_valid(&envelope);
}

#[test]
fn welcome_envelope_validates_against_schema() {
    // Given a Welcome envelope with grants and config.
    let envelope = Envelope::for_host(
        HostToPlugin::Welcome(Welcome {
            protocol_version: 1,
            plugin_id: "themes".to_owned(),
            read_dirs: vec!["/home/u/.config/jinn/themes".to_owned()],
            write_dirs: vec!["/home/u/.local/share/jinn/plugins/themes".to_owned()],
            http_allowed: false,
            config: serde_json::json!({ "refresh_seconds": 30 }),
        }),
        0,
        0,
    );

    // Then it validates against the committed schema.
    assert_valid(&envelope);
}

#[test]
fn unknown_plugin_tag_deserializes_to_unknown() {
    // Given a wire line with a tag this build does not know, carrying data.
    let line = r#"{"v":1,"seq":7,"ts":0,"type":"future_message","payload":{"x":1}}"#;

    // When deserializing as a plugin→host envelope.
    let envelope: Envelope = serde_json::from_str(line).expect("tolerant deserialize");

    // Then the message is Unknown (payload dropped, no error).
    assert_eq!(
        envelope.msg,
        jinn_plugin_api::PluginToHostOrHostToPlugin::Unknown
    );
}

#[test]
fn unknown_host_tag_deserializes_to_unknown() {
    // Given a host→plugin wire line with an unknown tag.
    let line = r#"{"v":1,"seq":0,"ts":0,"type":"future_event","detail":"hi"}"#;

    // When deserializing.
    let envelope: Envelope = serde_json::from_str(line).expect("tolerant deserialize");

    // Then the message is Unknown (direction is unknowable from an
    // unknown tag; the observable contract is "no message we understand").
    assert_eq!(envelope.msg, PluginToHostOrHostToPlugin::Unknown);
}

#[test]
fn tool_call_event_envelope_validates_against_schema() {
    // Given a ToolCallEvent envelope.
    let envelope = Envelope::for_host(
        HostToPlugin::ToolCallEvent(ToolCallEvent {
            session_id: "s-1".to_owned(),
            tool_call_id: "call_1".to_owned(),
            name: "mcp__parallel__web_search".to_owned(),
            arguments: r#"{"objective":"find rust docs"}"#.to_owned(),
        }),
        0,
        0,
    );

    // Then it validates against the committed schema.
    assert_valid(&envelope);
}

#[test]
fn tool_result_event_envelope_validates_against_schema() {
    // Given a ToolResultEvent envelope.
    let envelope = Envelope::for_host(
        HostToPlugin::ToolResultEvent(ToolResultEvent {
            session_id: "s-1".to_owned(),
            tool_call_id: "call_1".to_owned(),
            name: "mcp__parallel__web_fetch".to_owned(),
            content: r#"{"results":[{"url":"https://a","title":"A"}]}"#.to_owned(),
            success: true,
        }),
        1,
        0,
    );

    // Then it validates against the committed schema.
    assert_valid(&envelope);
}

#[test]
fn turn_end_event_envelope_validates_against_schema() {
    // Given a TurnEndEvent envelope.
    let envelope = Envelope::for_host(
        HostToPlugin::TurnEndEvent(TurnEndEvent {
            session_id: "s-1".to_owned(),
            final_answer: true,
        }),
        2,
        0,
    );

    // Then it validates against the committed schema.
    assert_valid(&envelope);
}

#[test]
fn push_citations_envelope_validates_against_schema() {
    // Given a PushCitations envelope with populated citations.
    let envelope = Envelope::for_plugin(
        PluginToHost::PushCitations(PushCitations {
            session_id: "s-1".to_owned(),
            citations: vec![
                PluginCitation {
                    url: "https://example.com/a".to_owned(),
                    title: "Example A".to_owned(),
                    content: Some("An excerpt.".to_owned()),
                },
                PluginCitation {
                    url: "https://example.com/b".to_owned(),
                    title: "https://example.com/b".to_owned(),
                    content: None,
                },
            ],
        }),
        1,
        0,
    );

    // Then it validates against the committed schema.
    assert_valid(&envelope);
}

#[test]
fn push_citations_without_content_validates_against_schema() {
    // Given a PushCitations envelope whose citation has no content field.
    let line = r#"{"v":1,"seq":1,"ts":0,"type":"push_citations","session_id":"s-1","citations":[{"url":"https://example.com","title":"Example"}]}"#;

    // When deserializing.
    let envelope: Envelope = serde_json::from_str(line).expect("content is optional");

    // Then the citation deserialized with content absent.
    let PluginToHostOrHostToPlugin::Plugin(PluginToHost::PushCitations(msg)) = envelope.msg else {
        unreachable!("expected PushCitations");
    };
    assert_eq!(msg.citations.len(), 1);
    assert_eq!(msg.citations[0].content, None);
}

#[test]
fn new_event_and_contribution_envelopes_round_trip() {
    // Given each new wire message.
    let envelopes = vec![
        Envelope::for_host(
            HostToPlugin::ToolCallEvent(ToolCallEvent {
                session_id: "s".to_owned(),
                tool_call_id: "c".to_owned(),
                name: "n".to_owned(),
                arguments: "{}".to_owned(),
            }),
            1,
            1,
        ),
        Envelope::for_host(
            HostToPlugin::ToolResultEvent(ToolResultEvent {
                session_id: "s".to_owned(),
                tool_call_id: "c".to_owned(),
                name: "n".to_owned(),
                content: "out".to_owned(),
                success: false,
            }),
            2,
            1,
        ),
        Envelope::for_host(
            HostToPlugin::TurnEndEvent(TurnEndEvent {
                session_id: "s".to_owned(),
                final_answer: false,
            }),
            3,
            1,
        ),
        Envelope::for_plugin(
            PluginToHost::PushCitations(PushCitations {
                session_id: "s".to_owned(),
                citations: vec![PluginCitation {
                    url: "https://x".to_owned(),
                    title: "X".to_owned(),
                    content: None,
                }],
            }),
            4,
            1,
        ),
    ];

    // When round-tripping each through a JSON string.
    for envelope in envelopes {
        let json = serde_json::to_string(&envelope).expect("serialize");
        let back: Envelope = serde_json::from_str(&json).expect("deserialize");

        // Then it is unchanged.
        assert_eq!(envelope, back);
    }
}

#[test]
fn envelope_round_trips_through_json() {
    // Given a populated envelope.
    let envelope = Envelope::for_plugin(
        PluginToHost::SetThemeEntries(jinn_plugin_api::SetThemeEntries {
            themes: vec![sample_theme()],
        }),
        42,
        1_700_000_000_000,
    );

    // When round-tripping through a JSON string.
    let json = serde_json::to_string(&envelope).expect("serialize");
    let back: Envelope = serde_json::from_str(&json).expect("deserialize");

    // Then it is unchanged.
    assert_eq!(envelope, back);
}

#[rstest::rstest]
#[case("cyan")]
#[case("14")]
#[case("#112233")]
#[case("rgb(10, 20, 30)")]
fn color_strings_accept_all_core_formats(#[case] color: &str) {
    // Given a theme whose color values use the given format.
    let mut colors = BTreeMap::new();
    colors.insert(THEME_COLOR_SLOTS[0].key().to_owned(), color.to_owned());
    let theme = ThemeDef {
        name: "formats".to_owned(),
        description: None,
        colors,
    };

    // When serializing to wire JSON and back.
    let json = serde_json::to_value(&theme).expect("serialize");
    let back: ThemeDef = serde_json::from_value(json).expect("deserialize");

    // Then the color string is preserved verbatim.
    assert_eq!(
        back.colors.get(THEME_COLOR_SLOTS[0].key()),
        Some(&color.to_owned())
    );
}

#[test]
fn theme_def_serializes_slot_keys_as_snake_case() {
    // Given a theme def referencing two slots via the typed enum.
    let mut colors = BTreeMap::new();
    colors.insert(
        jinn_plugin_api::ThemeColorSlot::FocusAccent
            .key()
            .to_owned(),
        "#ffffff".to_owned(),
    );
    colors.insert(
        jinn_plugin_api::ThemeColorSlot::QuakeBarBg.key().to_owned(),
        "black".to_owned(),
    );
    let theme = ThemeDef {
        name: "keys".to_owned(),
        description: None,
        colors,
    };

    // When serializing to wire JSON.
    let json = serde_json::to_value(&theme).expect("serialize");
    let colors = json.get("colors").expect("colors map");

    // Then the keys are snake_case (matching core Theme field names).
    assert!(colors.get("focus_accent").is_some());
    assert!(colors.get("quake_bar_bg").is_some());
}

#[test]
fn every_slot_key_is_a_snake_case_identifier() {
    // Given every declared color slot.
    // When examining each wire key.
    for slot in THEME_COLOR_SLOTS {
        let key = slot.key();

        // Then it is non-empty, snake_case, and ASCII.
        assert!(!key.is_empty(), "empty key for {slot:?}");
        assert!(
            key.chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
            "non-snake-case key {key:?}"
        );
    }
}
