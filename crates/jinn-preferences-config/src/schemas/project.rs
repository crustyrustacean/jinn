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

    use super::ProjectConfig;

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
