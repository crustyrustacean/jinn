#![allow(clippy::string_slice, reason = "test code")]
//! Smoke test: verify `toml_edit` preserves comments on round-trip.
//!
//! This is a standalone sanity check that the crate behaves as expected
//! before we build abstractions on top of it. Not part of any production
//! code path.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use toml_edit::DocumentMut;

const FIXTURE: &str = include_str!("../src/default_providers.toml");

#[rstest::rstest]
#[test]
fn round_trip_preserves_comments_when_patching_one_field() {
    // Given the shipped default providers.toml (which is comment-rich).
    let mut doc: DocumentMut = FIXTURE.parse().expect("parse fixture");

    // When we mutate a single field: change openai's api_key_env.
    let providers = doc["providers"].as_table_mut().expect("providers table");
    let openai = providers
        .get_mut("openai")
        .and_then(|item| item.as_table_mut())
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
        .find("[providers.openai]")
        .expect("find openai block");
    let openai_block_end = output[openai_block_start..]
        .find("\n\n")
        .map_or(output.len(), |i| openai_block_start + i);
    let openai_block = &output[openai_block_start..openai_block_end];
    assert!(openai_block.contains("MY_CUSTOM_KEY"));
    assert!(!openai_block.contains("OPENAI_API_KEY"));
}

#[rstest::rstest]
#[test]
fn map_keyed_provider_table_can_be_removed_by_key() {
    // Given a document with map-keyed [providers.<name>] tables.
    let mut doc: DocumentMut = FIXTURE.parse().expect("parse");

    // When we delete a provider by its map key.
    let providers = doc["providers"].as_table_mut().expect("providers table");
    let original_len = providers.len();
    providers
        .remove("deepseek")
        .expect("deepseek provider exists");

    // Then exactly one was removed.
    assert_eq!(providers.len(), original_len - 1);
    let output = doc.to_string();
    // And the surviving blocks still have their comments.
    assert!(output.contains("# jinn provider configuration"));
    // And the deleted provider's table is gone.
    assert!(!output.contains("[providers.deepseek]"));
}

/// The smoke test survives a CRLF checkout.
///
/// There is no `.gitattributes` in this repository, so a contributor on
/// Windows with `core.autocrlf=true` checks the fixture out with CRLF line
/// endings, and `include_str!` embeds *those* bytes. The round-trip must
/// still work: `toml_edit` normalizes the line endings while parsing, and
/// `str::lines` strips the carriage return, so the comment lines the test
/// looks for are the same either way.
///
/// This test pins that safety so a future change to the fixture handling
/// cannot reintroduce a Windows-only failure.
#[rstest::rstest]
#[test]
fn round_trip_preserves_comments_with_crlf_line_endings() {
    // Given the shipped default providers.toml rewritten with CRLF endings.
    let crlf = FIXTURE.replace('\n', "\r\n");

    // When it is parsed and one field is patched.
    let mut doc: DocumentMut = crlf.parse().expect("parse crlf fixture");
    let providers = doc["providers"].as_table_mut().expect("providers table");
    let openai = providers
        .get_mut("openai")
        .and_then(|item| item.as_table_mut())
        .expect("find openai");
    openai["api_key_env"] = toml_edit::value("MY_CUSTOM_KEY");
    let output = doc.to_string();

    // Then every original comment is still present.
    for original_line in crlf.lines().filter(|l| l.starts_with('#')) {
        assert!(
            output.contains(original_line),
            "expected comment line preserved under CRLF: {original_line:?}"
        );
    }

    // And the patch took effect.
    assert!(output.contains("api_key_env = \"MY_CUSTOM_KEY\""));
}
