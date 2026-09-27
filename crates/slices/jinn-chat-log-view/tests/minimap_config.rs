//! Minimap configuration: the band boundary and its absent-section default.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use jinn_preferences_config::schemas::MinimapConfig;

#[rstest::rstest]
fn default_minimap_config_has_positive_token_bound() {
    // Given default minimap config.
    let config = MinimapConfig::default();

    // When reading the band boundary.
    let max_tokens = config.max_tokens;

    // Then the band boundary is a positive token count (not pinned to a
    // specific value — that's the Default impl's choice, not a contract).
    assert!(max_tokens > 0);
}

#[rstest::rstest]
fn load_parses_minimap_config() {
    // Given a jinn.toml with a minimap section.
    let config = jinn_config::testutil::config_layer("[ui.minimap]\nmax_tokens = 5000\n");

    // When reading the section.
    let minimap = config.get::<MinimapConfig>().expect("minimap reads");

    // Then minimap config is parsed.
    assert_eq!(minimap.max_tokens, 5000);
}

#[rstest::rstest]
fn load_without_minimap_section_uses_defaults() {
    // Given a jinn.toml without a minimap section.
    let config = jinn_config::testutil::config_layer("last_model = \"ollama/llama3\"\n");

    // When reading the section.
    let minimap = config.get::<MinimapConfig>().expect("minimap reads");

    // Then minimap uses defaults.
    assert_eq!(minimap, MinimapConfig::default());
}
