//! Command policy - resolution and matching for the bash tool.
//!
//! Resolves the blocked-command rules that apply to a session's cwd: the
//! global rules from `jinn.toml`'s `[[tools.bash_command_policy]]`, chained ahead
//! of the rules of the configured project containing that cwd
//! (`~`-expanded lexical longest-prefix match). The result is compiled into a
//! matcher consulted by the bash tool before any child process spawns.
//!
//! Global-first ordering makes the global list a floor: since first match
//! wins, a project policy can add blocks but never lift a global one, and the
//! global rules still apply outside every configured project.
//!
//! Advisory-strength by design: rules exist to stop well-trained habits
//! (like `cargo test -p` in a whole-workspace repo), not to resist a
//! determined actor.

use std::path::{Path, PathBuf};

use regex::Regex;

use jinn_preferences_config::schemas::ProjectConfig;
use jinn_preferences_config::schemas::command_policy::CommandPolicyRule;

/// Compiled blocked-command rules for one project. Empty matches nothing.
#[derive(Debug, Clone, Default)]
pub struct CompiledCommandPolicy {
    /// Compiled regexes paired with their messages, in config order.
    rules: Vec<(Regex, String)>,
}

impl CompiledCommandPolicy {
    /// Compiles user-authored rules. An invalid regex is skipped with a
    /// `tracing::warn!` naming the pattern — one bad rule is inert, never
    /// fatal for the project or the tool call.
    #[must_use]
    pub fn compile(rules: &[CommandPolicyRule]) -> Self {
        let compiled: Vec<(Regex, String)> = rules
            .iter()
            .filter_map(|rule| match Regex::new(&rule.pattern) {
                Ok(regex) => Some((regex, rule.message.clone())),
                Err(err) => {
                    tracing::warn!(
                        pattern = %rule.pattern,
                        %err,
                        "command_policy: skipping rule with invalid regex"
                    );
                    None
                }
            })
            .collect();
        Self { rules: compiled }
    }

    /// Returns `(pattern_as_written, message)` for the first rule matching
    /// `command`. Config order is precedence: first match wins.
    #[must_use]
    pub fn matched_message(&self, command: &str) -> Option<(&str, &str)> {
        self.rules
            .iter()
            .find(|(regex, _)| regex.is_match(command))
            .map(|(regex, message)| (regex.as_str(), message.as_str()))
    }

    /// True when no rules are compiled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// Returns the command-policy rules that apply to `cwd`: the global rules
/// configured in `jinn.toml` chained ahead of the rules of the configured
/// project containing `cwd` (or an empty project tail when no project matches).
///
/// Global rules come first because config order is precedence (first match
/// wins) — a project policy can add blocks but never lift a global one.
#[must_use]
pub fn resolve_rules(
    config: &jinn_config::ConfigLayer,
    cwd: &Path,
    home: &Path,
) -> Vec<CommandPolicyRule> {
    let global = config.get_list::<CommandPolicyRule>().unwrap_or_default();
    let projects = config.get_list::<ProjectConfig>().unwrap_or_default();
    let project_rules =
        matching_project(&projects, cwd, home).map_or_else(Vec::new, |p| p.command_policy.clone());
    global.iter().chain(project_rules.iter()).cloned().collect()
}

/// Returns the configured project with the longest `~`-expanded path that is
/// a lexical prefix of (or equal to) `cwd`.
fn matching_project<'a>(
    projects: &'a [ProjectConfig],
    cwd: &Path,
    home: &Path,
) -> Option<&'a ProjectConfig> {
    projects
        .iter()
        .filter_map(|project| {
            let expanded = expand_tilde(&project.path, home);
            cwd.starts_with(&expanded)
                .then_some((expanded.as_os_str().len(), project))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, project)| project)
}

/// Expands a leading `~` (and `~/`) in a configured project path against `home`.
fn expand_tilde(path: &Path, home: &Path) -> PathBuf {
    let Some(first) = path.components().next() else {
        return path.to_path_buf();
    };
    match first {
        std::path::Component::Normal(marker) if marker == "~" => {
            let suffix: PathBuf = path.components().skip(1).collect();
            if suffix.as_os_str().is_empty() {
                home.to_path_buf()
            } else {
                home.join(suffix)
            }
        }
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use std::sync::Arc;

    use super::*;

    fn rule(pattern: &str, message: &str) -> CommandPolicyRule {
        CommandPolicyRule {
            pattern: pattern.to_owned(),
            message: message.to_owned(),
        }
    }

    fn project(path: &str, rules: Vec<CommandPolicyRule>) -> ProjectConfig {
        ProjectConfig {
            path: PathBuf::from(path),
            command_policy: rules,
        }
    }

    /// A layer holding `global` and `projects` under their umbrellas, so
    /// tests exercise resolution through the same read path production uses.
    fn layer_with(
        global: &[CommandPolicyRule],
        projects: &[ProjectConfig],
    ) -> jinn_config::ConfigLayer {
        // Both are `ConfigList` sections: real top-level arrays of tables,
        // not a wrapper table holding a sequence. That shape is what keeps
        // each entry's own comment attached to it across a save.
        let mut doc = String::new();
        if !global.is_empty() {
            for body in serialize_all(global) {
                doc.push_str("[[tools.bash_command_policy]]\n");
                doc.push_str(&body);
                doc.push('\n');
            }
        }
        if !projects.is_empty() {
            for body in serialize_all(projects) {
                doc.push_str("[[project.entry]]\n");
                doc.push_str(&body);
                doc.push('\n');
            }
        }
        let parsed = doc.parse().expect("test TOML parses");
        jinn_config::ConfigLayer::load(Arc::new(jinn_config::InMemoryConfigStorage::new(parsed)))
            .expect("layer loads")
    }

    /// Each value as the `key = value` lines of its own TOML table, so a
    /// hand-built document reads the way a user's file does rather than as
    /// one inline `{...}` per array-of-tables entry.
    fn serialize_all<T>(values: &[T]) -> Vec<String>
    where
        T: serde::Serialize,
    {
        values
            .iter()
            .map(|value| {
                let table = toml::Value::try_from(value)
                    .expect("value serializes")
                    .as_table()
                    .expect("value is a table")
                    .clone();
                table
                    .iter()
                    .map(|(k, v)| format!("{k} = {v}\n"))
                    .collect::<String>()
            })
            .collect()
    }

    fn resolve_for(
        global: &[CommandPolicyRule],
        projects: &[ProjectConfig],
        cwd: &str,
        home: &Path,
    ) -> Vec<CommandPolicyRule> {
        resolve_rules(&layer_with(global, projects), Path::new(cwd), home)
    }

    fn resolve_in(
        global: &[CommandPolicyRule],
        projects: &[ProjectConfig],
        cwd: &Path,
        home: &Path,
    ) -> Vec<CommandPolicyRule> {
        resolve_rules(&layer_with(global, projects), cwd, home)
    }

    #[rstest::rstest]
    #[case("/home/me/w/repo", true)]
    #[case("/home/me/w/repo/sub/dir", true)]
    #[case("/home/me/w/repo-sibling", false)]
    #[case("/home/me/w", false)]
    #[case("/elsewhere", false)]
    fn cwd_inside_project_matches(#[case] cwd: &str, #[case] expected: bool) {
        // Given a project configured with a tilde-prefixed path and a home dir.
        let projects = [project("~/w/repo", vec![rule("a", "m")])];
        let home = Path::new("/home/me");

        // When resolving rules for a cwd.
        let rules = resolve_for(&[], &projects, cwd, home);

        // Then membership follows the lexical prefix (component-wise).
        assert_eq!(rules.is_empty(), !expected);
    }

    #[rstest::rstest]
    #[test]
    #[rstest::rstest]
    fn tilde_only_path_expands_to_home_itself() {
        // Given a project configured as bare `~`.
        let projects = [project("~", vec![rule("a", "m")])];
        let home = Path::new("/home/me");

        // When resolving rules for a cwd directly inside home.
        let rules = resolve_for(&[], &projects, "/home/me/notes", home);

        // Then the tilde expanded to home and the rules apply.
        assert_eq!(rules.len(), 1);
    }

    #[rstest::rstest]
    #[test]
    fn longest_prefix_project_wins_over_ancestor() {
        // Given a nested pair of configured projects, each with a distinct rule.
        let projects = [
            project("/w", vec![rule("outer", "outer msg")]),
            project("/w/repo", vec![rule("inner", "inner msg")]),
        ];
        let cwd = Path::new("/w/repo/src");

        // When resolving rules for a cwd inside the inner project.
        let rules = resolve_in(&[], &projects, cwd, Path::new("/"));

        // Then the inner (longest prefix) project's rules win.
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pattern, "inner");
    }

    #[rstest::rstest]
    #[test]
    fn global_rule_applies_outside_every_project() {
        // Given a global rule and projects none of which contain the cwd.
        let global = [rule("forbidden", "global msg")];
        let projects = [project(
            "/w/repo",
            vec![rule("project-only", "project msg")],
        )];
        let cwd = Path::new("/elsewhere");

        // When resolving rules for a cwd outside every configured project.
        let rules = resolve_in(&global, &projects, cwd, Path::new("/"));

        // Then the global rule still applies.
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pattern, "forbidden");
    }

    #[rstest::rstest]
    #[test]
    fn global_rule_is_matched_before_a_matching_project_rule() {
        // Given a global and a project rule that both match the same command.
        let global = [rule("forbidden", "global msg")];
        let projects = [project("/w/repo", vec![rule("forbidden", "project msg")])];
        let cwd = Path::new("/w/repo/src");

        // When resolving rules for a cwd inside the project.
        let rules = resolve_in(&global, &projects, cwd, Path::new("/"));

        // Then first-match-wins picks the global rule's message.
        let policy = CompiledCommandPolicy::compile(&rules);
        assert_eq!(
            policy.matched_message("run forbidden now"),
            Some(("forbidden", "global msg"))
        );
    }

    #[rstest::rstest]
    #[test]
    fn project_rule_still_applies_with_no_global_policy() {
        // Given a project rule and an empty global policy.
        let projects = [project(
            "/w/repo",
            vec![rule("project-only", "project msg")],
        )];
        let cwd = Path::new("/w/repo/src");

        // When resolving rules for a cwd inside the project.
        let rules = resolve_in(&[], &projects, cwd, Path::new("/"));

        // Then the project rules are returned unchanged.
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pattern, "project-only");
    }

    #[rstest::rstest]
    #[test]
    fn invalid_global_regex_is_inert_while_a_global_sibling_enforces() {
        // Given a global policy with one invalid regex followed by a valid rule.
        let global = [
            rule("([unclosed", "never compiles"),
            rule("forbidden", "global msg"),
        ];

        // When compiling the resolved global rules.
        let policy = CompiledCommandPolicy::compile(&resolve_for(
            &global,
            &[],
            "/elsewhere",
            Path::new("/"),
        ));

        // Then the invalid rule is silently inert (warned at compile).
        assert!(policy.matched_message("([unclosed thing").is_none());
        // And the valid global rule still enforces.
        assert_eq!(
            policy.matched_message("run forbidden now"),
            Some(("forbidden", "global msg"))
        );
    }

    #[rstest::rstest]
    #[case("cargo test -p jinn-kernel", true)]
    #[case("cargo t -p foo", true)]
    #[case("cargo test", false)]
    #[case("rg \"cargo test\" notes.md", false)]
    fn dash_p_policy_matches_only_dash_p_commands(#[case] command: &str, #[case] expected: bool) {
        // Given a policy with the canonical `-p` guard regex.
        let policy =
            CompiledCommandPolicy::compile(&[rule(r"cargo\s+(test|t)\b.*\s-p\b", "use just test")]);

        // When matching a command.
        let matched = policy.matched_message(command);

        // Then matches follow the pattern's intent: `-p` forms are blocked,
        // plain and quoted forms are not.
        assert_eq!(matched.is_some(), expected, "command: {command}");
    }

    #[rstest::rstest]
    #[test]
    fn first_matching_rule_wins_in_config_order() {
        // Given a policy whose two rules both match the command.
        let policy = CompiledCommandPolicy::compile(&[
            rule("first", "first message"),
            rule("second", "second message"),
        ]);

        // When matching a command both rules match.
        let matched = policy.matched_message("first and second");

        // Then the first rule (config order) supplies pattern and message.
        assert_eq!(matched, Some(("first", "first message")));
    }

    #[rstest::rstest]
    #[test]
    fn empty_policy_matches_nothing() {
        // Given a policy compiled from no rules.
        let policy = CompiledCommandPolicy::default();

        // When matching any command.
        let matched = policy.matched_message("rm -rf /");

        // Then nothing matches and the policy is empty.
        assert!(matched.is_none());
        assert!(policy.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn invalid_regex_rule_is_inert_but_sibling_still_enforces() {
        // Given a policy with one invalid regex followed by one valid rule.
        let policy = CompiledCommandPolicy::compile(&[
            rule("([unclosed", "never compiles"),
            rule("forbidden", "blocked"),
        ]);

        // When matching a command the invalid rule would have caught.
        let invalid_hit = policy.matched_message("([unclosed thing");
        // And a command the valid rule catches.
        let valid_hit = policy.matched_message("run forbidden now");

        // Then the invalid rule is silently inert (warned at compile).
        assert!(invalid_hit.is_none());
        // And the valid rule still enforces.
        assert_eq!(valid_hit, Some(("forbidden", "blocked")));
    }
}
