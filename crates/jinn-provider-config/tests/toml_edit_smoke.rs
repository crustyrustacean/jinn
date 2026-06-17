#![allow(clippy::string_slice, reason = "test code")]
//! Smoke test: verify `toml_edit` preserves comments on round-trip.
//!
//! This is a standalone sanity check that the crate behaves as expected
//! before we build abstractions on top of it. Not part of any production
//! code path.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::items_after_statements,
    reason = "test code"
)]

use toml_edit::DocumentMut;

const FIXTURE: &str = include_str!("../src/default_providers.toml");

#[test]
fn round_trip_preserves_comments_when_patching_one_field() {
    // Given the shipped default providers.toml (which is comment-rich).
    let mut doc: DocumentMut = FIXTURE.parse().expect("parse fixture");

    // When we mutate a single field: change openai's api_key_env.
    let providers = doc["providers"]
        .as_array_of_tables_mut()
        .expect("providers");
    let openai = providers
        .iter_mut()
        .find(|t| t.get("name").and_then(|v| v.as_str()) == Some("openai"))
        .expect("find openai");
    openai["api_key_env"] = toml_edit::value("MY_CUSTOM_KEY");

    let output = doc.to_string();

    // Then all original comments are still present verbatim.
    for original_line in FIXTURE.lines().filter(|l| l.starts_with('#')) {
        assert!(
            output.contains(original_line),
            "expected comment line preserved: {original_line}"
        );
    }

    // And the patch took effect.
    assert!(output.contains("api_key_env = \"MY_CUSTOM_KEY\""));
    // And the original key value is gone from openai's block.
    // (Other providers also have api_key_env, so we check the specific block.)
    let openai_block_start = output
        .find("[[providers]]\nname = \"openai\"")
        .expect("find openai block");
    let openai_block_end = output[openai_block_start..]
        .find("\n\n")
        .map_or(output.len(), |i| openai_block_start + i);
    let openai_block = &output[openai_block_start..openai_block_end];
    assert!(openai_block.contains("MY_CUSTOM_KEY"));
    assert!(!openai_block.contains("OPENAI_API_KEY"));
}

#[test]
fn array_of_tables_can_be_matched_by_key_field() {
    // Given a document with multiple [[providers]] blocks.
    let mut doc: DocumentMut = FIXTURE.parse().expect("parse");

    // When we delete a provider by name.
    let providers = doc["providers"]
        .as_array_of_tables_mut()
        .expect("providers");
    let original_len = providers.len();
    providers.retain(|t| t.get("name").and_then(|v| v.as_str()) != Some("deepseek"));

    // Then exactly one was removed.
    assert_eq!(providers.len(), original_len - 1);
    let output = doc.to_string();
    // And the surviving blocks still have their comments.
    assert!(output.contains("# jinn provider configuration"));
    // And the deleted provider's name is gone.
    assert!(!output.contains("name = \"deepseek\""));
}
