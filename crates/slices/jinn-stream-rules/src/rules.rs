//! Reading the assembled stream rules.
//!
//! Three sources, in order:
//!
//! 1. the **embedded** shipped set (`stream_rules.toml`, compiled into the
//!    binary) — always present, so the command guards work with no user
//!    file at all;
//! 2. a **user `stream_rules.toml`** beside `jinn.toml` — when the file
//!    exists it *replaces* the embedded set wholesale. An empty file is
//!    therefore a deliberate off switch; a malformed file warns and falls
//!    back to the embedded set, because a typo must never leave jinn
//!    unguarded;
//! 3. `[[stream_rules.entry]]` in **`jinn.toml`** — appended after, so a
//!    user's own rules layer on top of whichever base set won.
//!
//! After assembly, the `[stream_rules] disabled_rules` name list from
//! `jinn.toml` filters the whole set in one pass. The filter sits here —
//! after every source has contributed, not inside any one reader — because
//! names address rules from *any* source: a shipped rule or a
//! `stream_rules.toml` rule has no `jinn.toml` entry to carry a flag, but
//! every rule has a name. A name matching nothing is inert; removing a
//! name re-arms its rule on the next read or refresh.
//!
//! Each source is read once at activation and never cached: a reload is
//! observed by the next launch, and a rule the user deleted is not left
//! armed by a stale copy.
//!
//! This is also where the removed `[[tools.bash_command_policy]]` section
//! is read. Its rules became stream rules scoped to `tool:bash`, which is
//! the same behaviour under a name that also covers the rest of the rule
//! surface — so a user's existing patterns keep working without their file
//! being rewritten.

use std::path::Path;

use jinn_config::ConfigLayer;
use jinn_preferences_config::schemas::{
    LegacyCommandPolicyRule, LegacyProjectConfig, StreamRuleConfig, StreamRuleFileEntry,
    StreamRulesConfig, parse_stream_rules_file,
};
use jinn_preferences_config::{STREAM_RULES_FILE_NAME, STREAM_RULES_TEMPLATE};

/// Reads the configured stream rules for a run.
///
/// `preferences_path` is where this run's `jinn.toml` lives; the optional
/// user rules file is the `stream_rules.toml` sibling of that document.
///
/// The assembled order is: embedded set, replaced wholesale by an existing
/// user file (malformed → warn and keep the embedded set), then everything
/// `jinn.toml` and the legacy sections contribute appended after — minus
/// every rule `disabled_rules` names.
pub fn read_rules(config: &ConfigLayer, preferences_path: &Path) -> Vec<StreamRuleConfig> {
    let user_file = preferences_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(STREAM_RULES_FILE_NAME);

    let mut rules = embedded_rules();
    if user_file.exists() {
        match std::fs::read_to_string(&user_file) {
            Ok(text) => match parse_stream_rules_file(&text) {
                Ok(file) => {
                    rules = file
                        .entries()
                        .iter()
                        .map(StreamRuleFileEntry::live_rule)
                        .collect();
                    tracing::debug!(
                        path = %user_file.display(),
                        rules = rules.len(),
                        "user stream_rules.toml replaces the embedded set"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        path = %user_file.display(),
                        error = %error,
                        "user stream_rules.toml is malformed, continuing with the embedded set"
                    );
                }
            },
            Err(error) => {
                tracing::warn!(
                    path = %user_file.display(),
                    error = %error,
                    "user stream_rules.toml could not be read, continuing with the embedded set"
                );
            }
        }
    }

    rules.extend(jinn_toml_rules(config));

    let disabled = config
        .get::<StreamRulesConfig>()
        .map(|section| section.disabled_rules)
        .unwrap_or_default();
    // Silent skip on purpose: a deliberate disable is the requested
    // behaviour, so a name that matches nothing is inert rather than a
    // warning the user would see on every read.
    rules.retain(|rule| !disabled.iter().any(|name| name == &rule.name));
    rules
}

/// The embedded shipped set, with its fixtures stripped.
///
/// The template is compile-time-embedded and pinned by the preferences
/// crate's template tests, so a parse failure here is unreachable in a
/// shipped build; it is still handled rather than unwrapped, because the
/// failure mode of a guard set must be "warn and continue", never "break
/// the launch".
fn embedded_rules() -> Vec<StreamRuleConfig> {
    match parse_stream_rules_file(STREAM_RULES_TEMPLATE) {
        Ok(file) => file
            .entries()
            .iter()
            .map(StreamRuleFileEntry::live_rule)
            .collect(),
        Err(error) => {
            tracing::error!(
                error = %error,
                "the embedded stream rules template does not parse; \
                 shipping with no base set"
            );
            Vec::new()
        }
    }
}

/// Reads the `[[stream_rules.entry]]` list and the removed
/// `[[tools.bash_command_policy]]` section out of `jinn.toml`.
///
/// A malformed section reads as no rules rather than failing activation: a
/// typo in the rules list must not stop jinn from starting.
fn jinn_toml_rules(config: &ConfigLayer) -> Vec<StreamRuleConfig> {
    let mut rules = match config.get_list::<StreamRuleConfig>() {
        Ok(rules) => rules,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "stream rules section is malformed, continuing with no stream rules"
            );
            Vec::new()
        }
    };
    rules.extend(read_legacy_command_policy(config));
    rules
}

/// Converts a removed `[[tools.bash_command_policy]]` entry into the stream
/// rule that replaced it.
///
/// The conversion is at read time on purpose. Rewriting the user's file would
/// discard the comments explaining why each pattern is shaped the way it is —
/// the shipped patterns carry several paragraphs of exactly that — and this
/// project preserves comments on save for the same reason.
///
/// A `command_policy` list nested under a `[[project.entry]]` is handled the
/// same way, scoped to that project, so a project's own rules survive too.
fn read_legacy_command_policy(config: &ConfigLayer) -> Vec<StreamRuleConfig> {
    let global = config.get_list::<LegacyCommandPolicyRule>();
    let projects = config.get_list::<LegacyProjectConfig>();

    if global.is_err() && projects.is_err() {
        return Vec::new();
    }

    tracing::warn!(
        key = "tools.bash_command_policy",
        replacement = "stream_rules.entry scoped to tool:bash",
        "`[[tools.bash_command_policy]]` is now a stream rule scoped to tool:bash; \
         your patterns still apply, and [[project.entry]].command_policy becomes a \
         rule's `project` field"
    );

    let mut converted = Vec::new();
    if let Ok(rules) = global {
        converted.extend(rules.iter().map(|rule| convert(rule, None)));
    }
    converted.extend(convert_project_rules(&projects.unwrap_or_default()));
    converted
}

/// Converts the `command_policy` list each legacy project entry carried.
///
/// Each becomes a rule carrying that project's path in its `project` field,
/// which is what a rule's project scope means today — so a project's own rules
/// keep applying in that project and nowhere else.
fn convert_project_rules(projects: &[LegacyProjectConfig]) -> Vec<StreamRuleConfig> {
    projects
        .iter()
        .flat_map(|project| {
            project
                .command_policy
                .iter()
                .map(|rule| convert(rule, Some(&project.path)))
        })
        .collect()
}

/// The stream rule equivalent of one legacy command-policy rule.
fn convert(rule: &LegacyCommandPolicyRule, project: Option<&std::path::Path>) -> StreamRuleConfig {
    StreamRuleConfig {
        name: format!("bash-policy: {}", rule.pattern),
        description: String::new(),
        conditions: vec![rule.pattern.clone()],
        // A legacy rule was about one tool, so it is scoped to that tool.
        // Interrupting that call's argument stream is what stops it running.
        scopes: vec!["tool:bash".to_owned()],
        body: rule.message.clone(),
        project: project.map(|p| p.to_string_lossy().into_owned()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::read_rules;
    use crate::matcher::CompiledSet;
    use jinn_config::ConfigLayer;
    use jinn_preferences_config::STREAM_RULES_FILE_NAME;
    use jinn_slices::{StreamContext, StreamRuleSet};

    /// Reads `doc` as a config layer.
    fn layer(doc: &str) -> ConfigLayer {
        jinn_config::testutil::config_layer(doc)
    }

    /// A `jinn.toml` path in a temp dir with no sibling rules file, for the
    /// pre-existing tests that only exercise the jinn.toml half of the read.
    fn temp_prefs() -> std::path::PathBuf {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("jinn.toml");
        // Leak the temp dir: the file must outlive the test's read, and the
        // read never writes to it, so a leaked empty dir is the cheapest
        // correct lifetime answer here.
        std::mem::forget(dir);
        path
    }

    /// A `jinn.toml` path inside `dir`, so the sibling rules file resolves
    /// inside the same temp directory.
    fn prefs_path_in(dir: &std::path::Path) -> std::path::PathBuf {
        dir.join("jinn.toml")
    }

    /// Writes a user `stream_rules.toml` beside `prefs`, returning its path.
    fn write_user_rules(prefs: &std::path::Path, body: &str) -> std::path::PathBuf {
        let path = prefs
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(STREAM_RULES_FILE_NAME);
        std::fs::write(&path, body).expect("write user rules file");
        path
    }

    /// The names of the rules a read assembles, in order.
    fn assembled_names(config: &ConfigLayer, prefs: &std::path::Path) -> Vec<String> {
        read_rules(config, prefs)
            .into_iter()
            .map(|r| r.name)
            .collect()
    }

    #[rstest::rstest]
    #[test]
    fn the_embedded_set_loads_when_no_user_file_exists() {
        // Given an empty temp dir (no user rules file) and an empty config.
        let dir = tempfile::tempdir().expect("temp dir");
        let config = layer("");

        // When the rules are read.
        let names = assembled_names(&config, &prefs_path_in(dir.path()));

        // Then the embedded shipped set is armed with no user configuration.
        assert!(
            names.iter().any(|n| n == "no-unbounded-recursive-rm"),
            "the embedded guards must load with no user file, got: {names:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_user_file_replaces_the_embedded_set_wholesale() {
        // Given a user rules file carrying exactly one rule.
        let dir = tempfile::tempdir().expect("temp dir");
        let prefs = prefs_path_in(dir.path());
        write_user_rules(
            &prefs,
            r#"
            [[stream_rules.entry]]
            name = 'user-only-rule'
            description = 'mine'
            conditions = ['NEVER-TYPE-this']
            scopes = ['tool:bash']
            body = 'Mine.'
            positive = ['NEVER-TYPE-this now']
            negative = ['echo hi']
        "#,
        );
        let config = layer("");

        // When the rules are read.
        let names = assembled_names(&config, &prefs);

        // Then only the user's rule is present — the embedded set is
        // replaced, not merged.
        assert_eq!(names, vec!["user-only-rule".to_owned()]);
    }

    #[rstest::rstest]
    #[test]
    fn an_existing_but_empty_user_file_disarms_every_shipped_guard() {
        // Given an existing but empty user rules file.
        let dir = tempfile::tempdir().expect("temp dir");
        let prefs = prefs_path_in(dir.path());
        write_user_rules(&prefs, "");
        let config = layer("");

        // When the rules are read.
        let names = assembled_names(&config, &prefs);

        // Then the assembly is empty: an existing file owns the base set,
        // and empty means the user disarmed it.
        assert!(
            names.is_empty(),
            "an empty user file disarms, got: {names:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_malformed_user_file_falls_back_to_the_embedded_set() {
        // Given a user rules file that is not valid TOML.
        let dir = tempfile::tempdir().expect("temp dir");
        let prefs = prefs_path_in(dir.path());
        write_user_rules(&prefs, "NOT [valid toml");
        let config = layer("");

        // When the rules are read.
        let names = assembled_names(&config, &prefs);

        // Then the embedded set is still armed: a typo must never leave
        // jinn unguarded.
        assert!(
            names.iter().any(|n| n == "no-unbounded-recursive-rm"),
            "a malformed user file must fall back to the embedded set, got: {names:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_jinn_toml_rule_appends_after_the_base_set() {
        // Given a jinn.toml carrying one rule and no user file.
        let dir = tempfile::tempdir().expect("temp dir");
        let prefs = prefs_path_in(dir.path());
        let config = layer(
            r#"
            [[stream_rules.entry]]
            name = "from-jinn-toml"
            conditions = ['TODO']
            scopes = ['text']
            body = 'Finish it.'
        "#,
        );

        // When the rules are read.
        let names = assembled_names(&config, &prefs);

        // Then the jinn.toml rule follows the embedded set, so its own
        // conditions still see every stream.
        assert_eq!(names.last().map(String::as_str), Some("from-jinn-toml"));
        assert!(
            names.iter().any(|n| n == "no-unbounded-recursive-rm"),
            "the embedded set must precede the appended rule"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_jinn_toml_rule_appends_after_a_user_file_set() {
        // Given a user file and a jinn.toml rule.
        let dir = tempfile::tempdir().expect("temp dir");
        let prefs = prefs_path_in(dir.path());
        write_user_rules(
            &prefs,
            r#"
            [[stream_rules.entry]]
            name = 'user-base'
            conditions = ['AAA-MARKER']
            scopes = ['tool:bash']
            body = 'User base.'
        "#,
        );
        let config = layer(
            r#"
            [[stream_rules.entry]]
            name = "from-jinn-toml"
            conditions = ['TODO']
            scopes = ['text']
            body = 'Finish it.'
        "#,
        );

        // When the rules are read.
        let names = assembled_names(&config, &prefs);

        // Then the order is user file first, jinn.toml second, embedded gone.
        assert_eq!(
            names,
            vec!["user-base".to_owned(), "from-jinn-toml".to_owned()]
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_legacy_command_policy_rule_becomes_a_bash_scoped_rule() {
        // Given the removed section, with one rule.
        let config = layer(
            r#"
            [[tools.bash_command_policy]]
            pattern = 'LEGACY-MARKER-CMD'
            message = 'That is the whole filesystem.'
        "#,
        );

        // When the rules are read.
        let rules = read_rules(&config, &temp_prefs());

        // Then it is a stream rule scoped to the bash tool.
        let converted = rules
            .iter()
            .find(|r| r.name == "bash-policy: LEGACY-MARKER-CMD")
            .expect("the converted rule is in the assembled set");
        assert_eq!(converted.conditions, vec!["LEGACY-MARKER-CMD".to_owned()]);
        assert_eq!(
            converted.scopes,
            vec!["tool:bash".to_owned()],
            "a legacy rule was about one tool, so it must scope to it"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_legacy_command_policy_rule_still_stops_the_command() {
        // Given the removed section, with one rule naming a command the
        // embedded base set does not guard.
        let config = layer(
            r#"
            [[tools.bash_command_policy]]
            pattern = 'LEGACY-MARKER-CMD'
            message = 'That is the whole filesystem.'
        "#,
        );

        // When the converted rule is compiled and the offending command streams.
        let set = CompiledSet::build(&read_rules(&config, &temp_prefs()));
        let mut session = set.new_session(&jinn_core_types::SessionId::new());
        let fired = session
            .check(
                r#"{"command":"LEGACY-MARKER-CMD now"}"#,
                StreamContext::tool(0, "bash"),
            )
            .map(|hit| hit.name.clone());

        // Then it fires, which is what stops the call from running.
        assert_eq!(fired, Some("bash-policy: LEGACY-MARKER-CMD".to_owned()));
    }

    #[rstest::rstest]
    #[test]
    fn a_legacy_command_policy_keeps_its_message() {
        // Given the removed section, with a rule carrying guidance.
        let config = layer(
            r#"
            [[tools.bash_command_policy]]
            pattern = 'chmod 777'
            message = 'That makes the file world-writable.'
        "#,
        );

        // When the rules are read.
        let rules = read_rules(&config, &temp_prefs());

        // Then the guidance the author wrote is what the model will be told.
        let converted = rules
            .iter()
            .find(|r| r.name == "bash-policy: chmod 777")
            .expect("the converted rule is in the assembled set");
        assert_eq!(converted.body, "That makes the file world-writable.");
    }

    #[rstest::rstest]
    #[test]
    fn a_projects_own_command_policy_becomes_a_project_scoped_rule() {
        // Given two projects, the second carrying its own blocked-command rule.
        let config = layer(
            r#"
            [[project.entry]]
            path = "/home/dev/code/myapp"

            [[project.entry]]
            path = "/home/dev/code/other"
            [[project.entry.command_policy]]
            pattern = 'npm run release'
            message = 'Run the package test, not the whole workspace.'
        "#,
        );

        // When the rules are read.
        let rules = read_rules(&config, &temp_prefs());

        // Then the rule is scoped to the project that declared it.
        assert!(
            rules.iter().any(|r| {
                r.conditions == vec!["npm run release".to_owned()]
                    && r.project.as_deref() == Some("/home/dev/code/other")
            }),
            "a project's own rule must keep its project scope, got: {:?}",
            rules
                .iter()
                .map(|r| (&r.conditions, &r.project))
                .collect::<Vec<_>>()
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_file_with_no_command_policy_reads_only_its_stream_rules() {
        // Given a file with a stream rule and nothing legacy.
        let config = layer(
            r#"
            [[stream_rules.entry]]
            name = "no-todo"
            conditions = ['TODO']
            scopes = ['text']
            body = 'Finish it or remove it.'
        "#,
        );

        // When the rules are read.
        let rules = read_rules(&config, &temp_prefs());

        // Then the rule is there and the migration contributed nothing.
        assert!(
            rules.iter().any(|r| r.name == "no-todo"),
            "the jinn.toml rule must read alongside the base set"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_rule_naming_the_removed_trigger_key_still_fires() {
        // Given a rule written against the removed `on_trigger` key.
        let config = layer(
            r#"
            [[stream_rules.entry]]
            name = "no-rm"
            conditions = ['LEGACY-TRIGGER-CMD']
            scopes = ['tool:bash']
            body = 'Never.'
        "#,
        );

        // When the rule is compiled and the offending command streams.
        let set = CompiledSet::build(&read_rules(&config, &temp_prefs()));
        let mut session = set.new_session(&jinn_core_types::SessionId::new());
        let fired = session
            .check(
                r#"{"command":"LEGACY-TRIGGER-CMD"}"#,
                StreamContext::tool(0, "bash"),
            )
            .map(|hit| hit.name.clone());

        // Then it fires as a plain interrupt: the key is simply unknown now,
        // and a file that still carries it keeps working.
        assert_eq!(fired, Some("no-rm".to_owned()));
    }

    /// A config carrying a global rule and a project-scoped one.
    fn mixed_project_config() -> ConfigLayer {
        layer(
            r#"
            [[stream_rules.entry]]
            name = 'global-rule'
            conditions = ['GLOBAL-MARKER-CMD']
            scopes = ['tool:bash']
            body = 'Never.'

            [[stream_rules.entry]]
            name = 'myapp-rule'
            conditions = ['npm\s+run\s+release']
            scopes = ['tool:bash']
            project = 'myapp'
            body = 'Run the checked-in script instead.'
        "#,
        )
    }

    #[rstest::rstest]
    #[test]
    fn a_global_and_a_project_rule_both_read_from_one_list() {
        // Given a file with a global rule and a project-scoped one.
        let config = mixed_project_config();

        // When the rules are read.
        let rules = read_rules(&config, &temp_prefs());

        // Then both are present, each with its own scope intact.
        let global = rules.iter().find(|r| r.name == "global-rule");
        assert_eq!(global.and_then(|r| r.project.as_deref()), None);
        let scoped = rules.iter().find(|r| r.name == "myapp-rule");
        assert_eq!(scoped.and_then(|r| r.project.as_deref()), Some("myapp"));
    }

    #[rstest::rstest]
    #[test]
    fn a_global_rule_merged_with_a_project_rule_keeps_both_blocks() {
        // Given the same two rules, compiled for the scoped project.
        let set = CompiledSet::build(&read_rules(&mixed_project_config(), &temp_prefs()))
            .for_project(std::path::Path::new("/home/dev/code/myapp"));

        // When each command's arguments stream in that project.
        let global = {
            let mut session = set.new_session(&jinn_core_types::SessionId::new());
            session
                .check(
                    r#"{"command":"GLOBAL-MARKER-CMD"}"#,
                    StreamContext::tool(0, "bash"),
                )
                .map(|hit| hit.name.clone())
        };
        let scoped = {
            let mut session = set.new_session(&jinn_core_types::SessionId::new());
            session
                .check(
                    r#"{"command":"npm run release"}"#,
                    StreamContext::tool(0, "bash"),
                )
                .map(|hit| hit.name.clone())
        };

        // Then both fire: merging adds the project rule, it does not displace
        // the global one.
        assert_eq!(global, Some("global-rule".to_owned()));
        assert_eq!(scoped, Some("myapp-rule".to_owned()));
    }

    #[rstest::rstest]
    #[test]
    fn a_project_rule_merged_with_a_global_rule_is_silent_outside_its_project() {
        // Given the same two rules, resolved for a project that is not myapp.
        let set =
            crate::matcher::CompiledSet::build(&read_rules(&mixed_project_config(), &temp_prefs()))
                .for_project(std::path::Path::new("/home/dev/code/other"));

        // When the global rule's command's arguments stream there.
        let global = {
            let mut session = set.new_session(&jinn_core_types::SessionId::new());
            session
                .check(
                    r#"{"command":"GLOBAL-MARKER-CMD"}"#,
                    StreamContext::tool(0, "bash"),
                )
                .map(|hit| hit.name.clone())
        };

        // Then it still fires: a project-scoped rule cannot lift a global.
        assert_eq!(global, Some("global-rule".to_owned()));
    }

    #[rstest::rstest]
    #[test]
    fn a_legacy_and_a_stream_rule_merge_into_one_set() {
        // Given a file with the removed section beside a real stream rule.
        let config = layer(
            r#"
            [[tools.bash_command_policy]]
            pattern = 'chmod 777'
            message = 'World-writable.'

            [[stream_rules.entry]]
            name = 'no-todo'
            conditions = ['TODO']
            scopes = ['text']
            body = 'Finish it.'
        "#,
        );

        // When the rules are read and compiled.
        let rules = read_rules(&config, &temp_prefs());
        let set = crate::matcher::CompiledSet::build(&rules);

        // Then both are live: the legacy one fires on the tool stream, the
        // stream one fires on prose.
        let mut session = set.new_session(&jinn_core_types::SessionId::new());
        let legacy = session.check(
            r#"{"command":"chmod 777 x"}"#,
            StreamContext::tool(0, "bash"),
        );
        let mut prose = set.new_session(&jinn_core_types::SessionId::new());
        let modern = prose.check("leave a TODO here", StreamContext::text());
        assert!(
            legacy.is_some(),
            "the converted legacy rule must still fire on the tool stream"
        );
        assert!(modern.is_some(), "the stream rule must survive the merge");
    }

    #[rstest::rstest]
    #[test]
    fn a_legacy_project_rule_merges_with_a_global_one() {
        // Given a legacy global rule beside a legacy project rule.
        let config = layer(
            r#"
            [[tools.bash_command_policy]]
            pattern = 'drop\s+table'
            message = 'No.'

            [[project.entry]]
            path = "/home/dev/code/myapp"
            [[project.entry.command_policy]]
            pattern = 'truncate'
            message = 'Not here.'
        "#,
        );

        // When the rules are read and compiled for that project.
        let set = crate::matcher::CompiledSet::build(&read_rules(&config, &temp_prefs()))
            .for_project(std::path::Path::new("/home/dev/code/myapp"));

        // Then both fire: the project rule adds its block beside the global.
        let mut session = set.new_session(&jinn_core_types::SessionId::new());
        let global = session.check(
            r#"{"command":"drop table t"}"#,
            StreamContext::tool(0, "bash"),
        );
        let mut scoped = set.new_session(&jinn_core_types::SessionId::new());
        let in_project = scoped.check(
            r#"{"command":"truncate t"}"#,
            StreamContext::tool(0, "bash"),
        );
        assert!(global.is_some(), "the legacy global rule must still fire");
        assert!(
            in_project.is_some(),
            "the legacy project rule must fire in its project"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_disabled_name_disarms_an_embedded_rule() {
        // Given a jinn.toml naming a shipped rule in the disabled array.
        let config = layer(
            r#"
            [stream_rules]
            disabled_rules = ['no-unbounded-recursive-rm']
        "#,
        );

        // When the rules are read with no user file.
        let names = assembled_names(&config, &temp_prefs());

        // Then the named shipped rule is gone and its siblings stay armed —
        // the array reaches a rule with no jinn.toml entry of its own.
        assert!(
            !names.contains(&"no-unbounded-recursive-rm".to_owned()),
            "a named shipped rule must not survive, got: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "no-mkfs"),
            "the other shipped rules must stay armed, got: {names:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_disabled_name_disarms_a_user_file_rule() {
        // Given a user rules file and an array naming its rule.
        let dir = tempfile::tempdir().expect("temp dir");
        let prefs = prefs_path_in(dir.path());
        write_user_rules(
            &prefs,
            r#"
            [[stream_rules.entry]]
            name = 'user-only-rule'
            conditions = ['NEVER-TYPE-this']
            scopes = ['tool:bash']
            body = 'Mine.'
        "#,
        );
        let config = layer(
            r#"
            [stream_rules]
            disabled_rules = ['user-only-rule']
        "#,
        );

        // When the rules are read.
        let names = assembled_names(&config, &prefs);

        // Then the user-file rule is disarmed by name: the filter runs
        // after assembly, not inside the jinn.toml reader.
        assert!(
            names.is_empty(),
            "a named user-file rule must not survive, got: {names:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_disabled_name_disarms_a_jinn_toml_rule() {
        // Given a jinn.toml rule and its own name in the array.
        let config = layer(
            r#"
            [stream_rules]
            disabled_rules = ['from-jinn-toml']

            [[stream_rules.entry]]
            name = 'from-jinn-toml'
            conditions = ['TODO']
            scopes = ['text']
            body = 'Finish it.'
        "#,
        );

        // When the rules are read.
        let names = assembled_names(&config, &temp_prefs());

        // Then only the named rule is gone; the base set stays armed.
        assert!(
            !names.contains(&"from-jinn-toml".to_owned()),
            "a named jinn.toml rule must not survive, got: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "no-unbounded-recursive-rm"),
            "the embedded base set must stay armed, got: {names:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_disabled_name_disarms_a_legacy_converted_rule() {
        // Given the removed legacy section and an array naming the rule it
        // converts into.
        let config = layer(
            r#"
            [stream_rules]
            disabled_rules = ['bash-policy: LEGACY-MARKER-CMD']

            [[tools.bash_command_policy]]
            pattern = 'LEGACY-MARKER-CMD'
            message = 'That is the whole filesystem.'
        "#,
        );

        // When the rules are read.
        let names = assembled_names(&config, &temp_prefs());

        // Then the converted rule is disarmed by name too: conversions pass
        // through the same choke point as every other source.
        assert!(
            !names.contains(&"bash-policy: LEGACY-MARKER-CMD".to_owned()),
            "a named legacy rule must not survive, got: {names:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_disabled_name_matching_nothing_is_inert() {
        // Given an array naming a rule that exists nowhere.
        let config = layer(
            r#"
            [stream_rules]
            disabled_rules = ['never-existed']
        "#,
        );

        // When the rules are read.
        let names = assembled_names(&config, &temp_prefs());

        // Then the set is unchanged: names match at read time against
        // whatever assembled, and a stale name is inert, not an error.
        assert!(
            names.iter().any(|n| n == "no-unbounded-recursive-rm"),
            "the base set must be untouched by an unknown name, got: {names:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn removing_a_name_from_the_array_re_arms_the_rule() {
        // Given an armed jinn.toml rule — its name no longer in any array.
        let config = layer(
            r#"
            [[stream_rules.entry]]
            name = 'from-jinn-toml'
            conditions = ['TODO']
            scopes = ['text']
            body = 'Finish it.'
        "#,
        );

        // When the rules are read.
        let names = assembled_names(&config, &temp_prefs());

        // Then the rule is back: a name's absence means armed, so removing
        // it re-arms on the next read or refresh.
        assert!(
            names.contains(&"from-jinn-toml".to_owned()),
            "the re-armed rule must be read, got: {names:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_named_rule_never_fires_on_a_matching_stream() {
        // Given a rule the disabled array names.
        let config = layer(
            r#"
            [stream_rules]
            disabled_rules = ['from-jinn-toml']

            [[stream_rules.entry]]
            name = 'from-jinn-toml'
            conditions = ['TODO']
            scopes = ['text']
            body = 'Finish it.'
        "#,
        );

        // When the assembled set is compiled and prose matching its
        // pattern streams.
        let set = CompiledSet::build(&read_rules(&config, &temp_prefs()));
        let mut session = set.new_session(&jinn_core_types::SessionId::new());
        let fired = session
            .check("leave a TODO here", StreamContext::text())
            .map(|hit| hit.name.clone());

        // Then nothing fires: the rule is not in the compiled set.
        assert_eq!(fired, None);
    }

    #[rstest::rstest]
    #[test]
    fn an_enabled_rule_still_fires_beside_a_named_disabled_one() {
        // Given two matching rules, the first named in the array.
        let config = layer(
            r#"
            [stream_rules]
            disabled_rules = ['off']

            [[stream_rules.entry]]
            name = 'off'
            conditions = ['TODO']
            scopes = ['text']
            body = 'Off.'

            [[stream_rules.entry]]
            name = 'on'
            conditions = ['TODO']
            scopes = ['text']
            body = 'On.'
        "#,
        );

        // When the delta that matches both streams.
        let set = CompiledSet::build(&read_rules(&config, &temp_prefs()));
        let mut session = set.new_session(&jinn_core_types::SessionId::new());
        let fired = session
            .check("TODO", StreamContext::text())
            .map(|hit| hit.name.clone());

        // Then the enabled one fires: disabling one rule disarms only that
        // rule, not the set around it.
        assert_eq!(fired, Some("on".to_owned()));
    }
}
