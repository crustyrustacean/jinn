//! The stream-rule matcher actor — keeps the installed matcher current.
//!
//! Activation compiles the configured rules and installs the matcher once.
//! This actor owns the same install after that: a [`RefreshStreamRuleMatcher`]
//! command (published by a configuration writer, today the stream-rule
//! picker's confirm) re-runs the identical read + compile + install path, so
//! the matcher a fresh stream resolves is built from what the configuration
//! now holds — no restart.
//!
//! The actor is the only mid-session writer of the matcher cell, which keeps
//! the single-writer story intact: activation writes it before any stream
//! runs, and this actor writes it after every confirmed configuration change.

use jinn_slices::StreamRules;
use jinn_slices::cell::TypedCell;
use jinn_stream_rules_msg::RefreshStreamRuleMatcher;
use trouper::actor::ActorPath;
use trouper::actor::{MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;

/// The matcher actor's static trouper path.
pub const STREAM_RULE_MATCHER_PATH: &str = "stream-rule-matcher";

/// The stream-rule matcher actor.
///
/// Rebuilds the compiled set from the live configuration layer and
/// reinstalls it into the matcher cell on every refresh command.
pub struct StreamRuleMatcherActor {
    cell: TypedCell<StreamRules>,
    config: jinn_config::ConfigLayer,
    preferences_path: std::path::PathBuf,
}

impl ServiceActor for StreamRuleMatcherActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "trait contract: start is never called (spawn uses start_with)"
    )]
    async fn start(
        _args: &trouper::json::Json,
    ) -> Result<Self, error_stack::Report<RegistryError>> {
        // Never called: the spawn helper injects the cell and the
        // configuration via `start_with`.
        Err(
            error_stack::IntoReport::into_report(RegistryError::InvalidSpec)
                .attach("StreamRuleMatcherActor is spawned via start_with"),
        )
    }
}

impl StreamRuleMatcherActor {
    /// Spawns the actor at its static path.
    ///
    /// The cell, the configuration layer, and the preferences path are the
    /// same three values activation installed the first matcher from — the
    /// refresh is that path re-run, nothing more.
    pub fn spawn(
        system: &ActorSystem,
        cell: TypedCell<StreamRules>,
        config: jinn_config::ConfigLayer,
        preferences_path: std::path::PathBuf,
    ) -> ActorPath {
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(ActorPath::new(STREAM_RULE_MATCHER_PATH))
            .start_with(move || {
                Box::pin(async move {
                    Ok(Self {
                        cell,
                        config,
                        preferences_path,
                    })
                })
            })
            .handles::<RefreshStreamRuleMatcher>()
            .start()
    }

    /// Handles a [`RefreshStreamRuleMatcher`] by rebuilding the matcher
    /// from the current configuration and reinstalling it.
    ///
    /// The read is the same point-of-use read activation made, so the
    /// rebuilt set observes every configuration write that landed before
    /// the command was published — including the picker's own, which is
    /// why the confirm publishes only after its writes succeed.
    pub fn handle_refresh(&self) {
        let rules = crate::rules::read_rules(&self.config, &self.preferences_path);
        crate::install_compiled_matcher(&self.cell, &rules);
    }
}

impl MsgHandler<RefreshStreamRuleMatcher> for StreamRuleMatcherActor {
    async fn handle(&mut self, _msg: &RefreshStreamRuleMatcher, _ctx: &mut MsgCtx<'_>) {
        self.handle_refresh();
    }
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
    use jinn_slices::StreamContext;

    /// A config layer holding one rule named `name`.
    fn layer_with_rule(name: &str) -> jinn_config::ConfigLayer {
        jinn_config::testutil::config_layer(&format!(
            r#"
            [[stream_rules.entry]]
            name = "{name}"
            conditions = ['REBUILD-MARKER-{name}']
            scopes = ['text']
            body = 'Finish it.'
            "#
        ))
    }

    /// A temp preferences path with no sibling rules file. Returns the
    /// owned dir so the caller keeps it alive as long as the path is in use.
    fn temp_prefs() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let path = dir.path().join("jinn.toml");
        (dir, path)
    }

    /// An actor over a fresh matcher cell.
    fn actor() -> (
        StreamRuleMatcherActor,
        TypedCell<StreamRules>,
        tempfile::TempDir,
    ) {
        let slices = jinn_slices::Slices::new();
        let cell = slices
            .register(jinn_slices::stream_rules_slot(), StreamRules::empty())
            .expect("a fresh registry never has the stream-rules slot taken");
        let (prefs_dir, prefs) = temp_prefs();
        let actor = StreamRuleMatcherActor {
            cell: cell.clone(),
            config: layer_with_rule("rebuildable"),
            preferences_path: prefs,
        };
        (actor, cell, prefs_dir)
    }

    /// Whether a prose delta naming the rule trips the installed matcher,
    /// probed the way production probes it: the delta runs through `check`,
    /// and the stream's end runs `finalize` — the pair a real response always
    /// performs.
    ///
    /// Without the finalize half, a rule matching the marker's tail would read
    /// as silent here while firing in production at stream completion.
    fn fires(cell: &TypedCell<StreamRules>, marker: &str) -> Option<String> {
        let set = cell.read().installed()?;
        let mut session = set.new_session(&jinn_core_types::SessionId::new());
        session
            .check(marker, StreamContext::text())
            .or_else(|| session.finalize())
            .map(|hit| hit.name.clone())
    }

    #[rstest::rstest]
    fn a_refresh_installs_the_configured_rules() {
        // Given an actor whose cell holds no matcher.
        let (actor, cell, _prefs_dir) = actor();
        assert!(cell.read().installed().is_none());

        // When a refresh command is handled.
        actor.handle_refresh();

        // Then the configured rule is live in the cell.
        assert_eq!(
            fires(&cell, "REBUILD-MARKER-rebuildable"),
            Some("rebuildable".to_owned())
        );
    }

    #[rstest::rstest]
    fn a_disabled_rule_is_gone_after_a_refresh() {
        // Given a cell whose matcher was refreshed with the rule armed.
        let (actor, cell, _prefs_dir) = actor();
        actor.handle_refresh();
        assert!(fires(&cell, "REBUILD-MARKER-rebuildable").is_some());

        // When the rule is disabled underneath the actor and a refresh
        // runs — the picker-confirm shape.
        actor
            .config
            .set_stream_rule_disabled(&actor.preferences_path, "rebuildable", true)
            .expect("the write lands");
        actor.handle_refresh();

        // Then the rule no longer fires: the rebuilt set left it out.
        assert_eq!(fires(&cell, "REBUILD-MARKER-rebuildable"), None);
    }

    #[rstest::rstest]
    fn a_re_enabled_rule_is_back_after_a_refresh() {
        // Given a cell whose matcher was refreshed with the rule disabled.
        let (actor, cell, _prefs_dir) = actor();
        actor
            .config
            .set_stream_rule_disabled(&actor.preferences_path, "rebuildable", true)
            .expect("the write lands");
        actor.handle_refresh();
        assert!(fires(&cell, "REBUILD-MARKER-rebuildable").is_none());

        // When the rule is re-armed and a refresh runs.
        actor
            .config
            .set_stream_rule_disabled(&actor.preferences_path, "rebuildable", false)
            .expect("the write lands");
        actor.handle_refresh();

        // Then the rule fires again — a refresh re-arms as well as disarms.
        assert_eq!(
            fires(&cell, "REBUILD-MARKER-rebuildable"),
            Some("rebuildable".to_owned())
        );
    }
}
