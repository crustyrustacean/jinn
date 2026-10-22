//! Assembling the full stream-rule set — every rule the matcher enforces.
//!
//! Four sources, in order:
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
//!    user's own rules layer on top of whichever base set won;
//! 4. the removed `[[tools.bash_command_policy]]` section, converted at
//!    read time into rules scoped to `tool:bash`.
//!
//! The assembled list is **deduped by name, last definition wins**: a
//! later source's same-name entry replaces the earlier one's, keeping the
//! first position — exactly the duplicate rule the matcher's
//! `CompiledSet` applied, moved here so every consumer of the set (the
//! matcher's read, the picker's list, the unknown-name validation) shows
//! and accepts the one definition that actually fires.
//!
//! The result is deliberately **unfiltered**: the persisted
//! `disabled_rules` name array does not apply here. Names address rules
//! from any source, so the filter belongs to the matcher's read alone —
//! a disabled rule stays in this set, which is what keeps it visible in
//! the picker and re-armable by name.
//!
//! This is also where the removed `[[tools.bash_command_policy]]` section
//! is read. Its rules became stream rules scoped to `tool:bash`, which is
//! the same behaviour under a name that also covers the rest of the rule
//! surface — so a user's existing patterns keep working without their file
//! being rewritten.
//!
//! Each source is read at call time and never cached: a reload is observed
//! by the next read, and a rule the user deleted is not left armed by a
//! stale copy.

use jinn_config::ConfigLayer;

use crate::schemas::{
    LegacyCommandPolicyRule, LegacyProjectConfig, StreamRuleConfig, StreamRuleFileEntry,
    parse_stream_rules_file,
};
use crate::{STREAM_RULES_FILE_NAME, STREAM_RULES_TEMPLATE};
use marked_path::{Absolute, MarkedPathAccess, MarkedPathBuf};

/// Assembles the full stream-rule set: the embedded shipped set, replaced
/// wholesale by an existing user `stream_rules.toml` (malformed → warn and
/// keep the embedded set), then everything `jinn.toml` and the legacy
/// sections contribute appended after — deduped by name, last definition
/// winning, and without the disabled-name filter.
///
/// `preferences_path` is where this run's `jinn.toml` lives; the optional
/// user rules file is the `stream_rules.toml` sibling of that document.
///
/// Three consumers share this one assembly so they can never disagree: the
/// matcher slice installs exactly this set (minus the disabled names), the
/// stream-rule picker lists exactly this set, and the storage boundary
/// validates a name against exactly this set.
///
/// # Panics
///
/// Panics if `preferences_path` has no parent or the parent rejects a
/// bare-name join, neither of which can happen for an absolute path: an
/// absolute path always names a parent directory, and a parent directory
/// cannot contribute a drive prefix or absolute segment to a join.
#[expect(
    clippy::expect_used,
    reason = "parent-of-absolute and bare-name joins on typed absolute bases are infallible by construction"
)]
#[must_use]
pub fn assemble_stream_rules(
    config: &ConfigLayer,
    preferences_path: &MarkedPathBuf<Absolute>,
) -> Vec<StreamRuleConfig> {
    let user_file = preferences_path
        .parent()
        .expect("preferences path is absolute, so it has a parent")
        .join(STREAM_RULES_FILE_NAME)
        .expect("absolute parent joins a bare filename");

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
                        path = %user_file,
                        rules = rules.len(),
                        "user stream_rules.toml replaces the embedded set"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        path = %user_file,
                        error = %error,
                        "user stream_rules.toml is malformed, continuing with the embedded set"
                    );
                }
            },
            Err(error) => {
                tracing::warn!(
                    path = %user_file,
                    error = %error,
                    "user stream_rules.toml could not be read, continuing with the embedded set"
                );
            }
        }
    }

    rules.extend(jinn_toml_rules(config));
    dedupe_by_name_last_wins(rules)
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
/// A malformed section reads as no rules rather than failing the read: a
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

/// Collapses the assembled list to one entry per name, last definition
/// winning, keeping the first position.
///
/// This is the duplicate rule the matcher's `CompiledSet` applied, moved to
/// the assembly so the warning fires where the dedupe happens — a duplicate
/// never reaches `CompiledSet` from this path, so compiling stays silent
/// about names this function already resolved. The order property matters:
/// the picker shows the assembled order, and it must agree with the order
/// the matcher fires in.
fn dedupe_by_name_last_wins(rules: Vec<StreamRuleConfig>) -> Vec<StreamRuleConfig> {
    let mut deduped: Vec<StreamRuleConfig> = Vec::new();
    for rule in rules {
        match deduped
            .iter_mut()
            .find(|existing| existing.name == rule.name)
        {
            Some(existing) => {
                tracing::warn!(
                    rule = %rule.name,
                    "duplicate stream rule name, the later entry replaces the earlier"
                );
                *existing = rule;
            }
            None => deduped.push(rule),
        }
    }
    deduped
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::assemble_stream_rules;
    use jinn_config::ConfigLayer;
    use marked_path::{Absolute, MarkedPathAccess, MarkedPathBuf};

    /// Reads `doc` as a config layer.
    fn layer(doc: &str) -> ConfigLayer {
        jinn_config::testutil::config_layer(doc)
    }

    /// A `jinn.toml` path in a temp dir with no sibling rules file, for the
    /// tests that only exercise the jinn.toml half of the assembly.
    ///
    /// Returns the owned dir so the caller keeps it alive as long as the
    /// path is in use.
    fn temp_prefs() -> (jinn_testutil::JinnTempDir, MarkedPathBuf<Absolute>) {
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("jinn.toml")
            .expect("temp root joins a bare name");
        (dir, path)
    }

    /// A `jinn.toml` path inside `dir`, so the sibling rules file resolves
    /// inside the same temp directory.
    fn prefs_path_in(dir: &MarkedPathBuf<Absolute>) -> MarkedPathBuf<Absolute> {
        dir.join("jinn.toml").expect("temp root joins a bare name")
    }

    /// Writes a user `stream_rules.toml` beside `prefs`, returning its path.
    fn write_user_rules(prefs: &MarkedPathBuf<Absolute>, body: &str) -> MarkedPathBuf<Absolute> {
        let path = prefs
            .parent()
            .expect("prefs path is absolute, so it has a parent")
            .join(crate::STREAM_RULES_FILE_NAME)
            .expect("absolute parent joins a bare filename");
        std::fs::write(&path, body).expect("write user rules file");
        path
    }

    /// The names of the rules an assembly produces, in order.
    fn assembled_names(config: &ConfigLayer, prefs: &MarkedPathBuf<Absolute>) -> Vec<String> {
        assemble_stream_rules(config, prefs)
            .into_iter()
            .map(|r| r.name)
            .collect()
    }

    #[rstest::rstest]
    #[test]
    fn the_embedded_set_loads_when_no_user_file_exists() {
        // Given an empty temp dir (no user rules file) and an empty config.
        let dir = jinn_testutil::tempdir();
        let config = layer("");

        // When the rules are assembled.
        let names = assembled_names(&config, &prefs_path_in(&dir.path));

        // Then the embedded shipped set is present with no user
        // configuration — the guards work out of the box.
        assert!(
            names.iter().any(|n| n == "no-unbounded-recursive-rm"),
            "the embedded guards must load with no user file, got: {names:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_user_file_replaces_the_embedded_set_wholesale() {
        // Given a user rules file carrying exactly one rule.
        let dir = jinn_testutil::tempdir();
        let prefs = prefs_path_in(&dir.path);
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

        // When the rules are assembled.
        let names = assembled_names(&config, &prefs);

        // Then only the user's rule is present — the embedded set is
        // replaced, not merged.
        assert_eq!(names, vec!["user-only-rule".to_owned()]);
    }

    #[rstest::rstest]
    #[test]
    fn an_existing_but_empty_user_file_disarms_every_shipped_guard() {
        // Given an existing but empty user rules file.
        let dir = jinn_testutil::tempdir();
        let prefs = prefs_path_in(&dir.path);
        write_user_rules(&prefs, "");
        let config = layer("");

        // When the rules are assembled.
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
        let dir = jinn_testutil::tempdir();
        let prefs = prefs_path_in(&dir.path);
        write_user_rules(&prefs, "NOT [valid toml");
        let config = layer("");

        // When the rules are assembled.
        let names = assembled_names(&config, &prefs);

        // Then the embedded set is still there: a typo must never leave
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
        let dir = jinn_testutil::tempdir();
        let prefs = prefs_path_in(&dir.path);
        let config = layer(
            r#"
            [[stream_rules.entry]]
            name = "from-jinn-toml"
            conditions = ['TODO']
            scopes = ['text']
            body = 'Finish it.'
        "#,
        );

        // When the rules are assembled.
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
        let dir = jinn_testutil::tempdir();
        let prefs = prefs_path_in(&dir.path);
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

        // When the rules are assembled.
        let names = assembled_names(&config, &prefs);

        // Then the order is user file first, jinn.toml second, embedded gone.
        assert_eq!(
            names,
            vec!["user-base".to_owned(), "from-jinn-toml".to_owned()]
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_name_in_the_user_file_and_jinn_toml_resolves_to_the_jinn_toml_definition() {
        // Given a user file and a jinn.toml entry sharing a name, each with
        // its own body.
        let dir = jinn_testutil::tempdir();
        let prefs = prefs_path_in(&dir.path);
        write_user_rules(
            &prefs,
            r#"
            [[stream_rules.entry]]
            name = 'shared-name'
            conditions = ['FILE-MARKER']
            scopes = ['text']
            body = 'From the file.'
        "#,
        );
        let config = layer(
            r#"
            [[stream_rules.entry]]
            name = "shared-name"
            conditions = ['JINN-MARKER']
            scopes = ['text']
            body = 'From jinn.toml.'
        "#,
        );

        // When the rules are assembled.
        let rules = assemble_stream_rules(&config, &prefs);

        // Then the name appears exactly once, carrying the jinn.toml
        // definition — the later source in assembly order, which is the one
        // the matcher enforces.
        let matches: Vec<&crate::schemas::StreamRuleConfig> =
            rules.iter().filter(|r| r.name == "shared-name").collect();
        assert_eq!(
            matches.len(),
            1,
            "a shared name must render once, got: {matches:?}"
        );
        assert_eq!(matches[0].body, "From jinn.toml.");
    }

    #[rstest::rstest]
    #[test]
    fn a_duplicate_name_within_one_source_keeps_the_later_entry() {
        // Given a jinn.toml carrying the same name twice with different
        // bodies.
        let config = layer(
            r#"
            [[stream_rules.entry]]
            name = "doubled"
            conditions = ['EARLIER']
            scopes = ['text']
            body = 'The earlier body.'

            [[stream_rules.entry]]
            name = "doubled"
            conditions = ['LATER']
            scopes = ['text']
            body = 'The later body.'
        "#,
        );

        // When the rules are assembled.
        let (_prefs_dir, prefs) = temp_prefs();
        let rules = assemble_stream_rules(&config, &prefs);

        // Then one entry survives and it is the later definition — the
        // file's last word on the name is the one that fires.
        let matches: Vec<&crate::schemas::StreamRuleConfig> =
            rules.iter().filter(|r| r.name == "doubled").collect();
        assert_eq!(
            matches.len(),
            1,
            "a duplicated name must collapse, got: {matches:?}"
        );
        assert_eq!(matches[0].body, "The later body.");
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

        // When the rules are assembled.
        let (_prefs_dir, prefs) = temp_prefs();
        let rules = assemble_stream_rules(&config, &prefs);

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
    fn a_legacy_command_policy_keeps_its_message() {
        // Given the removed section, with a rule carrying guidance.
        let config = layer(
            r#"
            [[tools.bash_command_policy]]
            pattern = 'chmod 777'
            message = 'That makes the file world-writable.'
        "#,
        );

        // When the rules are assembled.
        let (_prefs_dir, prefs) = temp_prefs();
        let rules = assemble_stream_rules(&config, &prefs);

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
        let dir = jinn_testutil::tempdir();
        let myapp = dir
            .path
            .join("dev/code/myapp")
            .expect("temp root joins a bare name");
        let other = dir
            .path
            .join("dev/code/other")
            .expect("temp root joins a bare name");
        // The paths ride through `toml_edit::Value` so the interpolated text
        // is quoted and escaped, never spliced raw into the document.
        let config = layer(&format!(
            r#"
            [[project.entry]]
            path = {}

            [[project.entry]]
            path = {}
            [[project.entry.command_policy]]
            pattern = 'npm run release'
            message = 'Run the package test, not the whole workspace.'
        "#,
            toml_edit::Value::from(myapp.to_string_lossy().into_owned()),
            toml_edit::Value::from(other.to_string_lossy().into_owned()),
        ));

        // When the rules are assembled.
        let (_prefs_dir, prefs) = temp_prefs();
        let rules = assemble_stream_rules(&config, &prefs);

        // Then the rule is scoped to the project that declared it.
        assert!(
            rules.iter().any(|r| {
                r.conditions == vec!["npm run release".to_owned()]
                    && r.project.as_deref() == Some(other.to_string_lossy().as_ref())
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

        // When the rules are assembled.
        let (_prefs_dir, prefs) = temp_prefs();
        let rules = assemble_stream_rules(&config, &prefs);

        // Then the rule is there and the migration contributed nothing.
        assert!(
            rules.iter().any(|r| r.name == "no-todo"),
            "the jinn.toml rule must read alongside the base set"
        );
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
            project = '**/myapp'
            body = 'Run the checked-in script instead.'
        "#,
        )
    }

    #[rstest::rstest]
    #[test]
    fn a_global_and_a_project_rule_both_read_from_one_list() {
        // Given a file with a global rule and a project-scoped one.
        let config = mixed_project_config();

        // When the rules are assembled.
        let (_prefs_dir, prefs) = temp_prefs();
        let rules = assemble_stream_rules(&config, &prefs);

        // Then both are present, each with its own scope intact.
        let global = rules.iter().find(|r| r.name == "global-rule");
        assert_eq!(global.and_then(|r| r.project.as_deref()), None);
        let scoped = rules.iter().find(|r| r.name == "myapp-rule");
        assert_eq!(scoped.and_then(|r| r.project.as_deref()), Some("**/myapp"));
    }
}
