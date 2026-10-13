//! The stream-rules slice — regex rules tested against the live assistant stream.
//!
//! This slice contributes the matcher cell — registered by the catalog and
//! resolved here — into which the compiled rule set is installed, and the
//! matcher actor that re-installs it mid-session.
//!
//! The consumer is the inference actor's stream loop, which resolves the
//! matcher from the registry and asks it about each delta before publishing
//! that delta downstream. It never names this crate — it speaks
//! [`jinn_slices::StreamRuleSet`], which is why the vocabulary lives in
//! `jinn-slices` rather than here.
//!
//! # One writer, two moments
//!
//! The matcher cell has exactly one writer: this slice. Activation installs
//! the compiled set before the first stream runs, and the matcher actor
//! ([`matcher_actor::StreamRuleMatcherActor`]) re-runs the identical
//! read + compile + install when a [`jinn_stream_rules_msg::RefreshStreamRuleMatcher`]
//! command arrives — published by a configuration writer (the stream-rule
//! picker's confirm) so a confirmed toggle reaches the live matcher without
//! a restart.
//!
//! # What the slice does *not* do
//!
//! It does not intercept the stream itself. Interruption is the inference
//! loop's job; this slice only answers the question "does this delta trip a
//! rule, and which one".

pub mod matcher;
pub mod matcher_actor;
pub mod rules;

use std::path::Path;
use std::sync::Arc;

use jinn_slices::{RenderFacts, SliceHost, StreamRules};

use crate::matcher::CompiledSet;
use crate::rules::read_rules;

/// Compiles `rules` and installs the matcher into the slice's cell.
///
/// Every configured rule that fails to compile is warned and skipped by the
/// compile; a configuration with no usable rule installs an empty set, which
/// the stream loop treats as "no rules" and costs nothing per delta.
///
/// Shared by activation (the first install) and the matcher actor (every
/// refresh), so the two moments cannot drift apart.
fn install_compiled_matcher(
    cell: &jinn_slices::cell::TypedCell<StreamRules>,
    rules: &[jinn_preferences_config::schemas::StreamRuleConfig],
) {
    let set = Arc::new(CompiledSet::build(rules));
    cell.update(|payload| payload.install(set));
}

/// Activates the slice: assembles the rule set (embedded shipped set,
/// replaced by a user `stream_rules.toml` beside the preferences file,
/// plus `[[stream_rules.entry]]` from `jinn.toml`), compiles the rules,
/// installs the matcher into its cell, and spawns the matcher actor that
/// re-installs it on a refresh command.
///
/// `preferences_path` is where this run's `jinn.toml` lives — the user
/// rules file is its `stream_rules.toml` sibling.
///
/// The spawn must complete before the first publish can happen: subscribe
/// registers at spawn, and the picker cannot publish a refresh before the
/// user confirms a toggle, which is long after boot.
///
/// # Panics
///
/// Panics if the catalog has not run — the cell was minted there, so a
/// failure here means the boot list's ordering was violated.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(
    host: &mut SliceHost<'_, RenderFacts>,
    config: &jinn_config::ConfigLayer,
    preferences_path: &Path,
) {
    // Read at the point of use rather than cached, matching the rules
    // themselves: a reload is observed by the next refresh or launch.
    let rules = read_rules(config, preferences_path);

    // The cell was minted by the catalog before any activation ran, so a
    // failure here means the boot list's ordering was violated.
    let cell = host
        .slices()
        .reader::<StreamRules>(&jinn_slices::stream_rules_slot())
        .expect("the cell catalog registers the stream-rules slot before any slice activates");

    install_compiled_matcher(&cell, &rules);

    // The refresh listener: the same cell, config, and path, handed to the
    // actor that re-runs this install when a confirm publishes its command.
    matcher_actor::StreamRuleMatcherActor::spawn(
        host.system(),
        cell,
        config.clone(),
        preferences_path.to_path_buf(),
    );
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test module, panics are acceptable"
    )]
    use super::*;
    use jinn_preferences_config::StreamRulesStorage;

    #[rstest::rstest]
    #[tokio::test]
    async fn activate_installs_the_matcher_and_the_actor_reinstalls_on_a_refresh() {
        // Given a registry seeded like production boot seeds it, an
        // activated slice, and a config layer holding one armed rule.
        let slices = jinn_slices::Slices::new();
        jinn_cell_catalog::register_all_cells(&slices);
        let cell: jinn_slices::cell::TypedCell<StreamRules> = slices
            .reader(&jinn_slices::stream_rules_slot())
            .expect("the catalog registers the stream-rules slot");
        let config = jinn_config::testutil::config_layer(
            r#"
            [[stream_rules.entry]]
            name = "refreshable"
            conditions = ['REFRESH-MARKER']
            scopes = ['text']
            body = 'Finish it.'
            "#,
        );
        let prefs = {
            let dir = tempfile::tempdir().expect("temp dir");
            let path = dir.path().join("jinn.toml");
            std::mem::forget(dir);
            path
        };
        let system = trouper::system::ActorSystem::new(trouper::system::SystemConfig::production());
        let mut viewport = jinn_slices::view::Viewport::new();
        let overlay_views = jinn_slices::OverlayViews::new();
        let key_routes = jinn_slices::KeyRoutes::new();
        let mut host = jinn_slices::SliceHost::new(
            &slices,
            &mut viewport,
            &overlay_views,
            &key_routes,
            &system,
        );

        crate::activate(&mut host, &config, &prefs);

        // When the confirmed-toggle shape runs: the rule is disabled in the
        // configuration, then the refresh command is published on the bus
        // the actor subscribes to.
        config
            .set_stream_rule_disabled("refreshable", true)
            .expect("the write lands");
        system
            .publish(jinn_stream_rules_msg::RefreshStreamRuleMatcher)
            .await;

        // Then the live matcher honors it without a restart: the disabled
        // rule no longer fires on a stream that matches its pattern.
        let fired = {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                if let Some(set) = cell.read().installed() {
                    let mut session = set.new_session(&jinn_core_types::SessionId::new());
                    let fired = session
                        .check("REFRESH-MARKER", jinn_slices::StreamContext::text())
                        .map(|hit| hit.name.clone());
                    if fired.is_none() {
                        break fired;
                    }
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "the refresh never reached the matcher cell"
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        assert_eq!(
            fired, None,
            "a confirmed disable must disarm the live matcher"
        );
    }
}
