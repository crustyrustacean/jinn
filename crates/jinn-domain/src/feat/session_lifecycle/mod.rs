//! Session lifecycle management - setup/teardown command templates for sessions.
//!
//! Provides [`CommandTemplate`] for parsing and rendering shell command strings
//! that contain positional parameters (`$1`, `$2`, `$@`). Used by session
//! lifecycle recipes to bootstrap and tear down working directories.

pub mod arg_input_state;
pub mod builtin;
pub mod command_runner;
pub mod command_template;
pub mod intent;
pub mod picker_entry;
pub mod protocol;
pub mod render;

use serde::{Deserialize, Serialize};

/// A named session lifecycle recipe — paired setup and teardown commands.
///
/// Defined in `jinn.toml` under `[[session_lifecycle]]`. The setup command
/// runs when creating a new session; the teardown command runs when closing it.
/// Commands may contain positional parameters (`$1`, `$2`) that are collected
/// from the user before execution.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionLifecycle {
    /// Human-readable name shown in the lifecycle picker.
    pub name: String,
    /// Optional description shown below the name in the picker.
    #[serde(default)]
    pub description: Option<String>,
    /// Command to run when creating a session. Last line of stdout becomes the CWD.
    /// May contain `$1`, `$2` positional args. `None` means no setup (blank lifecycle).
    ///
    /// Supports both shell commands and builtin handlers.
    /// See [`LifecycleCommand`] for details.
    #[serde(rename = "setup_command", default)]
    pub setup: Option<builtin::LifecycleCommand>,
    /// Command to run when closing a session. Receives the same args as setup.
    /// `None` means no teardown needed.
    ///
    /// Supports both shell commands and builtin handlers.
    /// See [`LifecycleCommand`] for details.
    #[serde(rename = "teardown_command", default)]
    pub teardown: Option<builtin::LifecycleCommand>,
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use tempfile::TempDir;

    use super::SessionLifecycle;
    use crate::common::app_info::PREFS_FILE_NAME;
    use crate::feat::preferences_actor::user_preferences::{
        load_preferences_from, save_preferences_to, UserPreferences,
    };

    #[rstest::rstest]
    fn save_then_load_round_trips_session_lifecycles() {
        // Given preferences with a session lifecycle.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            session_lifecycles: vec![SessionLifecycle {
                name: "fossil branch".to_owned(),
                description: Some("Open a fossil branch in a new workdir".to_owned()),
                setup: Some(super::builtin::LifecycleCommand::Shell(
                    "~/.config/jinn/scripts/fossil-branch.sh $1".to_owned(),
                )),
                teardown: Some(super::builtin::LifecycleCommand::Shell(
                    "~/.config/jinn/scripts/fossil-cleanup.sh $1".to_owned(),
                )),
            }],
            ..UserPreferences::default()
        };

        // When saving and reloading.
        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");

        // Then the lifecycle is preserved.
        assert_eq!(reloaded.session_lifecycles.len(), 1);
        assert_eq!(reloaded.session_lifecycles[0].name, "fossil branch");
        assert!(matches!(
            reloaded.session_lifecycles[0].setup,
            Some(super::builtin::LifecycleCommand::Shell(ref s)) if s == "~/.config/jinn/scripts/fossil-branch.sh $1"
        ));
    }

    #[rstest::rstest]
    fn load_parses_table_array_session_lifecycle() {
        // Given a TOML file using [[session_lifecycle]] table array syntax.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"last_model = "ollama/llama3"

[[session_lifecycle]]
name = "fossil branch"
description = "Open a fossil branch in a new workdir"
setup_command = "~/.config/jinn/scripts/fossil-branch.sh $1"
teardown_command = "~/.config/jinn/scripts/fossil-cleanup.sh $1"
"#,
        )
        .expect("write");

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then session_lifecycles is populated.
        assert_eq!(prefs.session_lifecycles.len(), 1);
        assert_eq!(prefs.session_lifecycles[0].name, "fossil branch");
        assert!(matches!(
            prefs.session_lifecycles[0].setup,
            Some(super::builtin::LifecycleCommand::Shell(ref s)) if s == "~/.config/jinn/scripts/fossil-branch.sh $1"
        ));
    }

    #[rstest::rstest]
    fn save_preferences_preserves_session_lifecycle_block_and_comments() {
        // Given a jinn.toml with a session_lifecycle block.
        let original = "# my custom lifecycle\n[[session_lifecycle]]\nname = \"fossil-branch\"\ndescription = \"open a branch\"\n";
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, original).expect("write");

        // When loading and re-saving without changes.
        let prefs = load_preferences_from(&path).expect("load");
        save_preferences_to(&prefs, &path).expect("save");

        // Then the comment and entry are preserved.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(written.contains("# my custom lifecycle"));
        assert!(written.contains("name = \"fossil-branch\""));
    }

    #[rstest::rstest]
    fn save_preferences_deletes_session_lifecycle_block_on_struct_removal() {
        // Given a jinn.toml with two lifecycle blocks.
        let original = "# keep\n[[session_lifecycle]]\nname = \"alpha\"\n\n# delete\n[[session_lifecycle]]\nname = \"beta\"\n";
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, original).expect("write");

        // When loading and saving with only alpha kept.
        let mut prefs = load_preferences_from(&path).expect("load");
        prefs.session_lifecycles.retain(|l| l.name == "alpha");
        save_preferences_to(&prefs, &path).expect("save");

        // Then beta's block (and its comment) is removed.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(written.contains("# keep"));
        assert!(written.contains("name = \"alpha\""));
        assert!(!written.contains("beta"));
        assert!(!written.contains("# delete"));
    }

    #[rstest::rstest]
    fn save_preferences_appends_new_session_lifecycle_at_end() {
        // Given a jinn.toml with one lifecycle block.
        let original = "# existing\n[[session_lifecycle]]\nname = \"alpha\"\n";
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, original).expect("write");

        // When loading and adding a new lifecycle.
        let mut prefs = load_preferences_from(&path).expect("load");
        prefs.session_lifecycles.push(SessionLifecycle {
            name: "beta".to_owned(),
            ..Default::default()
        });
        save_preferences_to(&prefs, &path).expect("save");

        // Then beta appears after alpha.
        let written = std::fs::read_to_string(&path).expect("read");
        let alpha_pos = written.find("name = \"alpha\"").expect("alpha");
        let beta_pos = written.find("name = \"beta\"").expect("beta");
        assert!(alpha_pos < beta_pos);
        assert!(written.contains("# existing"));
    }
}
