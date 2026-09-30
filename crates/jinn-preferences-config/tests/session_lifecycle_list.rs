//! `[[session_lifecycle.script]]` round-trips through the configuration
//! layer.
//!
//! The list-of-tables spelling is the one users hand-write, so the layer must
//! read it back with comments, ordering, and every field intact. These cases
//! live with the schema they exercise rather than with the kernel that reads
//! it, because what they pin is the config layer's patch-and-reread behavior
//! for this section's array key.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code"
)]

use jinn_common::app_info::PREFS_FILE_NAME;
use jinn_config::{ConfigLayer, FilesystemConfigStorage};
use jinn_preferences_config::schemas::SessionLifecycle;
use tempfile::TempDir;

/// A layer over the temp `jinn.toml` the test just wrote.
fn layer_for(path: &std::path::Path) -> ConfigLayer {
    ConfigLayer::load(std::sync::Arc::new(FilesystemConfigStorage::new(
        path.to_path_buf(),
    )))
    .expect("layer loads")
}

#[rstest::rstest]
fn load_parses_table_array_session_lifecycle() {
    // Given a jinn.toml using the [[session_lifecycle.script]]
    // table array syntax.
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(PREFS_FILE_NAME);
    std::fs::write(
        &path,
        r#"last_model = "ollama/llama3"

[[session_lifecycle.script]]
name = "fossil branch"
description = "Open a fossil branch in a new workdir"
setup_command = "~/.config/jinn/scripts/fossil-branch.sh $1"
teardown_command = "~/.config/jinn/scripts/fossil-cleanup.sh $1"
"#,
    )
    .expect("write");
    let config = layer_for(&path);

    // When reading the lifecycle list.
    let lifecycles = config
        .get_list::<SessionLifecycle>()
        .expect("lifecycle list reads");

    // Then session_lifecycles is populated.
    assert_eq!(lifecycles.len(), 1);
    assert_eq!(lifecycles[0].name, "fossil branch");
    assert!(matches!(
        lifecycles[0].setup,
        Some(jinn_preferences_config::schemas::LifecycleCommand::Shell(ref s)) if s == "~/.config/jinn/scripts/fossil-branch.sh $1"
    ));
}

#[rstest::rstest]
fn put_lifecycle_list_preserves_session_lifecycle_block_and_comments() {
    // Given a jinn.toml with a session_lifecycle block.
    let original = "# my custom lifecycle\n[[session_lifecycle.script]]\nname = \"fossil-branch\"\ndescription = \"open a branch\"\n";
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(PREFS_FILE_NAME);
    std::fs::write(&path, original).expect("write");
    let config = layer_for(&path);

    // When re-saving the same list through the layer.
    let lifecycles = config
        .get_list::<SessionLifecycle>()
        .expect("lifecycle list reads");
    config
        .put_list::<SessionLifecycle>(&lifecycles)
        .expect("lifecycle list writes");

    // Then the comment and entry are preserved.
    let written = std::fs::read_to_string(&path).expect("read");
    assert!(written.contains("# my custom lifecycle"));
    assert!(written.contains("name = \"fossil-branch\""));
}

#[rstest::rstest]
fn put_lifecycle_list_deletes_session_lifecycle_block_on_entry_removal() {
    // Given a jinn.toml with two lifecycle blocks.
    let original = "# keep\n[[session_lifecycle.script]]\nname = \"alpha\"\n\n# delete\n[[session_lifecycle.script]]\nname = \"beta\"\n";
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(PREFS_FILE_NAME);
    std::fs::write(&path, original).expect("write");
    let config = layer_for(&path);

    // When saving with only alpha kept.
    let mut lifecycles = config
        .get_list::<SessionLifecycle>()
        .expect("lifecycle list reads");
    lifecycles.retain(|l| l.name == "alpha");
    config
        .put_list::<SessionLifecycle>(&lifecycles)
        .expect("lifecycle list writes");

    // Then beta's block (and its comment) is removed.
    let written = std::fs::read_to_string(&path).expect("read");
    assert!(written.contains("# keep"));
    assert!(written.contains("name = \"alpha\""));
    assert!(!written.contains("beta"));
    assert!(!written.contains("# delete"));
}

#[rstest::rstest]
fn put_lifecycle_list_appends_new_session_lifecycle_at_end() {
    // Given a jinn.toml with one lifecycle block.
    let original = "# existing\n[[session_lifecycle.script]]\nname = \"alpha\"\n";
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(PREFS_FILE_NAME);
    std::fs::write(&path, original).expect("write");
    let config = layer_for(&path);

    // When adding a new lifecycle and saving the list.
    let mut lifecycles = config
        .get_list::<SessionLifecycle>()
        .expect("lifecycle list reads");
    lifecycles.push(SessionLifecycle {
        name: "beta".to_owned(),
        ..Default::default()
    });
    config
        .put_list::<SessionLifecycle>(&lifecycles)
        .expect("lifecycle list writes");

    // Then beta appears after alpha.
    let written = std::fs::read_to_string(&path).expect("read");
    let alpha_pos = written.find("name = \"alpha\"").expect("alpha");
    let beta_pos = written.find("name = \"beta\"").expect("beta");
    assert!(alpha_pos < beta_pos);
    assert!(written.contains("# existing"));
}
