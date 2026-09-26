//! Smoke test: verify the regex auto-prune rules round-trip through the
//! configuration layer as an array of tables.
//!
//! Pairs with the `toml_edit` round-trip smoke test in the
//! `jinn-provider-config` crate.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::items_after_statements,
    reason = "test code"
)]

use jinn_preferences_config::schemas::AutoPruneConfig;
use jinn_preferences_config::schemas::auto_prune::RegexAutoPruneConfig;

#[rstest::rstest]
#[test]
fn auto_prune_regex_rules_round_trips_as_array_of_tables() {
    // Given a TOML snippet using the [[context_curation.auto_prune.regex.rules]]
    // form (which is what the codebase documents).
    let toml_str = r#"
        [context_curation.auto_prune.regex]
        enabled = true

        [[context_curation.auto_prune.regex.rules]]
        pattern = "foo"
        tool_name = "bash"
        keep_last = 3

        [[context_curation.auto_prune.regex.rules]]
        pattern = "bar"
    "#;

    // When reading the auto-prune section through the layer.
    let layer = jinn_config::testutil::config_layer(toml_str);
    let cfg = layer.get::<AutoPruneConfig>().expect("section reads").regex;

    // Then both rules are present and key fields preserved.
    assert_eq!(cfg.rules.len(), 2);
    assert_eq!(cfg.rules[0].pattern, "foo");
    assert_eq!(cfg.rules[0].keep_last, 3);
    assert_eq!(cfg.rules[1].pattern, "bar");
    assert_eq!(cfg.rules[1].tool_name, "bash"); // default applied

    // And re-serializing the bare struct round-trips through parse.
    let reserialized = toml::to_string(&cfg).expect("serialize");
    let reparsed: RegexAutoPruneConfig = toml::from_str(&reserialized).expect("reparse");
    assert_eq!(reparsed.rules.len(), 2);
}
