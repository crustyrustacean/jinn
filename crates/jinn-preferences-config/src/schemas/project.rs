//! The `[project]` umbrella — the curated project list.
//!
//! A list of tables at the top level of its umbrella, so the element type
//! declares itself with `ConfigList`: `[[project.entry]]` stays a list
//! and the layer matches entries by their identity field.
//!
//! The global command policy used to live here as
//! `[[project.global_command_policy]]`. It now lives under the umbrella
//! that owns it, `[[tools.bash_command_policy]]`.

use std::path::PathBuf;

use super::command_policy::CommandPolicyRule;
use serde::{Deserialize, Serialize};

impl jinn_config::ConfigList for ProjectConfig {
    const KEY: &'static str = "project.entry";
    const ENTRY_KEY: &'static str = "path";
    const ENTRY_FIELDS: &'static [&'static str] = &["path", "command_policy"];
}

/// A curated project directory shown in the project picker.
///
/// Defined in `jinn.toml` under `[[project.entry]]`. The `path` field
/// is the array key the patcher matches entries by, so add/remove
/// operations target a single table without disturbing siblings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// The absolute (or `~`-prefixed) directory path.
    pub path: PathBuf,
    /// Blocked-command rules the bash tool enforces for commands whose cwd
    /// falls inside this project. Empty means no policy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_policy: Vec<CommandPolicyRule>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use std::sync::Arc;

    use jinn_config::{ConfigLayer, ConfigList, InMemoryConfigStorage};

    use super::{CommandPolicyRule, ProjectConfig};

    #[rstest::rstest]
    fn project_entries_read_from_the_umbrella_key() {
        // Given a document listing two projects under the umbrella.
        let doc = r#"
            [[project.entry]]
            path = "/tmp/a"

            [[project.entry]]
            path = "/tmp/b"
        "#
        .parse()
        .expect("test TOML parses");
        let layer = ConfigLayer::load(Arc::new(InMemoryConfigStorage::new(doc))).expect("load");

        // When reading the list.
        let projects = layer.get_list::<ProjectConfig>().expect("list reads");

        // Then both entries are read, in document order.
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].path.to_string_lossy(), "/tmp/a");
        assert_eq!(projects[1].path.to_string_lossy(), "/tmp/b");
    }

    #[rstest::rstest]
    fn an_absent_project_list_reads_empty() {
        // Given a document with no project umbrella at all.
        let doc = "[tools]\ndefault_timeout_secs = 60"
            .parse()
            .expect("test TOML parses");
        let layer = ConfigLayer::load(Arc::new(InMemoryConfigStorage::new(doc))).expect("load");

        // When reading the list.
        let projects = layer.get_list::<ProjectConfig>().expect("list reads");

        // Then it is empty rather than an error.
        assert!(projects.is_empty());
    }

    #[rstest::rstest]
    fn a_projects_command_policy_renders_inline_and_reads_back() {
        // Given a project carrying two blocked-command rules.
        let project = ProjectConfig {
            path: "/tmp/demo".parse().expect("path parses"),
            command_policy: vec![
                CommandPolicyRule {
                    pattern: "git push".to_owned(),
                    message: "ask first".to_owned(),
                },
                CommandPolicyRule {
                    pattern: "rm -rf".to_owned(),
                    message: "never".to_owned(),
                },
            ],
        };
        let storage = Arc::new(InMemoryConfigStorage::new(
            "[tools]\nx = 1\n".parse().expect("parses"),
        ));
        let layer = ConfigLayer::load(storage.clone()).expect("load");

        // When saving it, then saving the identical value again.
        layer
            .put_list::<ProjectConfig>(std::slice::from_ref(&project))
            .expect("first save");
        let once = storage.text();
        layer
            .put_list::<ProjectConfig>(std::slice::from_ref(&project))
            .expect("second save");
        let twice = storage.text();

        // Then the rules render as one `key = value` on the entry rather than
        // as a `[[project.entry.command_policy]]` sub-list, which TOML reads
        // as a sibling of the project rather than part of it.
        assert!(
            once.contains("command_policy = [{ message = \"ask first\", pattern = \"git push\" }"),
            "policy not inline:\n{once}"
        );
        assert!(
            !once.contains("[[project.entry.command_policy]]"),
            "policy rendered as a sub-list:\n{once}"
        );

        // And an identical re-save does not move the file.
        assert_eq!(
            once, twice,
            "document moved:\nonce:\n{once}\ntwice:\n{twice}"
        );

        // And both rules read back off the right project.
        let read = layer.get_list::<ProjectConfig>().expect("read");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].command_policy, project.command_policy);
    }

    #[rstest::rstest]
    fn the_two_project_sections_do_not_share_a_key() {
        // Given the two lists under the project umbrella.
        // When their keys are compared.
        // Then they are distinct, so neither shadows the other.
        assert_ne!(
            <ProjectConfig as ConfigList>::KEY,
            crate::schemas::command_policy::GLOBAL_COMMAND_POLICY_KEY
        );
    }
}
