//! Auto-prune workers — background history pruning strategies.
//!
//! Each worker implements [`HistoryWorker`] and inspects conversation history
//! to identify stale or redundant entries that can be excluded from LLM context.
//!
//! # Adding a new auto-prune strategy
//!
//! 1. Create a new `foo.rs` file in this module.
//! 2. Implement [`HistoryWorker`] for your strategy struct.
//! 3. Spawn a [`HistoryWorkerActor`] with your worker in `actor_wiring.rs`.
//!
//! [`HistoryWorker`]: crate::feat::history_worker::worker_trait::HistoryWorker
//! [`HistoryWorkerActor`]: crate::feat::history_worker::actor::HistoryWorkerActor

pub mod anchor_shield;
pub mod anchored_assistant;
pub mod broken_edit;
pub mod consecutive_reads;
pub mod double_edit;
pub mod edit_read;
pub mod entry_token_cache;
pub(crate) mod min_age;
pub mod read_edit;
pub mod regex;
pub mod todo_prune;
pub mod tool_age_window;
pub mod trivial_assistant;
pub(crate) use min_age::is_within_min_age;

pub use anchor_shield::AnchorShieldAutoPruneWorker;
pub use anchored_assistant::AnchoredAssistantAutoPruneWorker;
pub use broken_edit::BrokenEditAutoPruneWorker;
pub use consecutive_reads::ConsecutiveReadsAutoPruneWorker;
pub use double_edit::DoubleEditAutoPruneWorker;
pub use edit_read::EditReadAutoPruneWorker;
pub use entry_token_cache::{
    HistoryWorkerChatEntryTokenCache, HistoryWorkerChatEntryTokenCacheEvictionActor,
    HistoryWorkerChatEntryTokenCacheEvictionActorDeps,
};
pub use read_edit::ReadEditAutoPruneWorker;
pub use regex::RegexAutoPruneWorker;
pub use todo_prune::TodoAutoPruneWorker;
pub use tool_age_window::ToolAgeWindowAutoPruneWorker;
pub use trivial_assistant::TrivialAssistantAutoPruneWorker;

// ── Aggregate config ────────────────────────────────────────────────────
//
// `AutoPruneConfig` groups every worker-specific config into one struct so
// `UserPreferences` can carry a single `[auto_prune]` table. The child configs
// live in their respective worker files (co-located with the workers that
// consume them); this aggregate just re-assembles them for serialization.
//
// A top-level scalar (`accumulation_threshold_tokens`) is also carried here:
// it's a global gate, not per-worker, so it lives on the aggregate.


/// Default accumulation threshold (in tokens) at which buffered pruner
/// context-override mutations flush.
const DEFAULT_ACCUMULATION_THRESHOLD_TOKENS: u32 = 10_000;

/// Serde default for [`AutoPruneConfig::accumulation_threshold_tokens`].
fn default_accumulation_threshold_tokens() -> u32 {
    DEFAULT_ACCUMULATION_THRESHOLD_TOKENS
}

use serde::{Deserialize, Serialize};

// Re-import the child config types from their co-located homes so the
// aggregate fields below resolve. All of these are `pub` in their own modules.
use crate::feat::auto_prune_worker::{
    anchor_shield::AnchorShieldConfig, anchored_assistant::AnchoredAssistantAutoPruneConfig,
    broken_edit::BrokenEditAutoPruneConfig, consecutive_reads::ConsecutiveReadsAutoPruneConfig,
    double_edit::DoubleEditAutoPruneConfig, edit_read::EditReadAutoPruneConfig,
    read_edit::ReadEditAutoPruneConfig, regex::RegexAutoPruneConfig,
    todo_prune::TodoAutoPruneConfig, tool_age_window::ToolAgeWindowAutoPruneConfig,
    trivial_assistant::TrivialAssistantAutoPruneConfig,
};

/// Auto-prune configuration.
///
/// Serialized as `[auto_prune]` in `jinn.toml`.
/// Groups all auto-prune strategy configurations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoPruneConfig {
    /// Edit-read auto-prune strategy configuration.
    #[serde(default)]
    pub edit_read: EditReadAutoPruneConfig,
    /// Read-edit auto-prune strategy configuration.
    #[serde(default)]
    pub read_edit: ReadEditAutoPruneConfig,
    /// Regex-based auto-prune strategy configuration.
    #[serde(default)]
    pub regex: RegexAutoPruneConfig,
    /// Broken-edit auto-prune strategy configuration.
    #[serde(default)]
    pub broken_edit: BrokenEditAutoPruneConfig,
    /// Todo auto-prune strategy configuration.
    #[serde(default)]
    pub todo: TodoAutoPruneConfig,
    /// Double-edit auto-prune strategy configuration.
    #[serde(default)]
    pub double_edit: DoubleEditAutoPruneConfig,
    /// Consecutive-reads auto-prune strategy configuration.
    #[serde(default)]
    pub consecutive_reads: ConsecutiveReadsAutoPruneConfig,
    /// Tool-age-window auto-prune strategy configuration.
    #[serde(default)]
    pub tool_age_window: ToolAgeWindowAutoPruneConfig,
    /// Trivial-assistant auto-prune strategy configuration.
    #[serde(default)]
    pub trivial_assistant: TrivialAssistantAutoPruneConfig,
    /// Anchored-assistant auto-prune strategy configuration.
    #[serde(default)]
    pub anchored_assistant: AnchoredAssistantAutoPruneConfig,
    /// Anchor-shield auto-prune strategy configuration.
    #[serde(default)]
    pub anchor_shield: AnchorShieldConfig,
    /// Token threshold at which accumulated pruner context-override mutations flush.
    ///
    /// Pruner `SetContextOverride` mutations are held in a per-session buffer until
    /// their deduplicated token total reaches this value, reducing server-side
    /// KV-cache rebuilds from frequent small prunes. Default: 10 000.
    #[serde(default = "default_accumulation_threshold_tokens")]
    pub accumulation_threshold_tokens: u32,
}


impl Default for AutoPruneConfig {
    fn default() -> Self {
        Self {
            edit_read: EditReadAutoPruneConfig::default(),
            read_edit: ReadEditAutoPruneConfig::default(),
            regex: RegexAutoPruneConfig::default(),
            broken_edit: BrokenEditAutoPruneConfig::default(),
            todo: TodoAutoPruneConfig::default(),
            double_edit: DoubleEditAutoPruneConfig::default(),
            consecutive_reads: ConsecutiveReadsAutoPruneConfig::default(),
            tool_age_window: ToolAgeWindowAutoPruneConfig::default(),
            trivial_assistant: TrivialAssistantAutoPruneConfig::default(),
            anchored_assistant: AnchoredAssistantAutoPruneConfig::default(),
            anchor_shield: AnchorShieldConfig::default(),
            accumulation_threshold_tokens: DEFAULT_ACCUMULATION_THRESHOLD_TOKENS,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use tempfile::TempDir;

    use crate::common::app_info::PREFS_FILE_NAME;
    use crate::feat::auto_prune_worker::{
        AutoPruneConfig,
        anchor_shield::AnchorShieldConfig,
        anchored_assistant::AnchoredAssistantAutoPruneConfig,
        broken_edit::BrokenEditAutoPruneConfig,
        consecutive_reads::ConsecutiveReadsAutoPruneConfig,
        double_edit::DoubleEditAutoPruneConfig,
        edit_read::EditReadAutoPruneConfig,
        read_edit::ReadEditAutoPruneConfig,
        regex::{RegexAutoPruneConfig, default_regex_min_age},
        todo_prune::TodoAutoPruneConfig,
        tool_age_window::ToolAgeWindowAutoPruneConfig,
        trivial_assistant::TrivialAssistantAutoPruneConfig,
    };
    use crate::feat::preferences_actor::user_preferences::{
        UserPreferences, load_preferences_from, save_preferences_to,
    };

    #[rstest::rstest]
    fn default_auto_prune_config_has_defaults() {
        let config = AutoPruneConfig::default();
        assert!(config.edit_read.enabled);
        assert_eq!(config.edit_read.min_age, 50);
        assert_eq!(config.read_edit.threshold, 2);
        assert!(config.read_edit.enabled);
        assert!(config.todo.enabled);
        assert!(config.consecutive_reads.enabled);
        assert_eq!(config.consecutive_reads.keep_last, 5);
        assert_eq!(config.consecutive_reads.min_age, 80);
        assert!(config.tool_age_window.enabled);
        assert_eq!(config.read_edit.min_age, 50);
        assert_eq!(config.double_edit.min_age, 20);
        assert_eq!(config.tool_age_window.min_age, 150);
        assert!(config.trivial_assistant.enabled);
        assert_eq!(config.trivial_assistant.min_age, 100);
        assert_eq!(config.trivial_assistant.max_tokens, 80);
        assert_eq!(config.accumulation_threshold_tokens, 10_000);
    }

    #[rstest::rstest]
    fn load_parses_auto_prune_read_edit_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, "[auto_prune.read_edit]\nenabled = false\n").expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(!prefs.auto_prune.read_edit.enabled);
    }

    #[rstest::rstest]
    fn load_parses_auto_prune_consecutive_reads_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "[auto_prune.consecutive_reads]\nenabled = false\nkeep_last = 5\n",
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(!prefs.auto_prune.consecutive_reads.enabled);
        assert_eq!(prefs.auto_prune.consecutive_reads.keep_last, 5);
    }

    #[rstest::rstest]
    fn load_parses_auto_prune_tool_age_window_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "[auto_prune.tool_age_window]\nenabled = false\nmin_age = 50\n",
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(!prefs.auto_prune.tool_age_window.enabled);
        assert_eq!(prefs.auto_prune.tool_age_window.min_age, 50);
    }

    #[rstest::rstest]
    fn load_parses_auto_prune_read_edit_min_age() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, "[auto_prune.read_edit]\nmin_age = 25\n").expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert_eq!(prefs.auto_prune.read_edit.min_age, 25);
    }

    #[rstest::rstest]
    fn load_parses_auto_prune_double_edit_min_age() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, "[auto_prune.double_edit]\nmin_age = 15\n").expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert_eq!(prefs.auto_prune.double_edit.min_age, 15);
    }

    #[rstest::rstest]
    fn config_without_min_age_uses_new_defaults() {
        // Given a TOML file with auto_prune sections that omit `min_age`, the new
        // defaults should kick in (50 for read_edit, 20 for double_edit, 100 for
        // tool_age_window).
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "[auto_prune.read_edit]\nenabled = true\n\n[auto_prune.double_edit]\nenabled = true\n\n[auto_prune.tool_age_window]\nenabled = true\n",
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert_eq!(prefs.auto_prune.read_edit.min_age, 50);
        assert_eq!(prefs.auto_prune.read_edit.threshold, 2);
        assert_eq!(prefs.auto_prune.double_edit.min_age, 20);
        assert_eq!(prefs.auto_prune.tool_age_window.min_age, 150);
    }

    #[rstest::rstest]
    fn save_then_load_round_trips_auto_prune_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            auto_prune: AutoPruneConfig {
                edit_read: EditReadAutoPruneConfig {
                    enabled: true,
                    min_age: 30,
                },
                read_edit: ReadEditAutoPruneConfig {
                    enabled: false,
                    min_age: 25,
                    threshold: 3,
                },
                regex: RegexAutoPruneConfig::default(),
                broken_edit: BrokenEditAutoPruneConfig {
                    enabled: false,
                    min_age: 3,
                },
                todo: TodoAutoPruneConfig {
                    enabled: false,
                    min_age: 0,
                },
                double_edit: DoubleEditAutoPruneConfig::default(),
                consecutive_reads: ConsecutiveReadsAutoPruneConfig::default(),
                tool_age_window: ToolAgeWindowAutoPruneConfig {
                    enabled: false,
                    min_age: 7,
                },
                trivial_assistant: TrivialAssistantAutoPruneConfig {
                    enabled: false,
                    min_age: 50,
                    max_tokens: 40,
                },
                anchored_assistant: AnchoredAssistantAutoPruneConfig {
                    enabled: false,
                    radius: 42,
                    min_age: 0,
                },
                anchor_shield: AnchorShieldConfig {
                    enabled: true,
                    radius: 20,
                },
                accumulation_threshold_tokens: 9999,
            },
            ..UserPreferences::default()
        };

        save_preferences_to(&prefs, &path).expect("save");

        let reloaded = load_preferences_from(&path).expect("load");
        assert!(reloaded.auto_prune.edit_read.enabled);
        assert_eq!(reloaded.auto_prune.edit_read.min_age, 30);
        assert!(!reloaded.auto_prune.read_edit.enabled);
        assert_eq!(reloaded.auto_prune.read_edit.min_age, 25);
        assert_eq!(reloaded.auto_prune.read_edit.threshold, 3);
        assert!(!reloaded.auto_prune.broken_edit.enabled);
        assert_eq!(reloaded.auto_prune.broken_edit.min_age, 3);
        assert!(!reloaded.auto_prune.todo.enabled);
        assert!(!reloaded.auto_prune.tool_age_window.enabled);
        assert_eq!(reloaded.auto_prune.tool_age_window.min_age, 7);
        assert!(!reloaded.auto_prune.trivial_assistant.enabled);
        assert_eq!(reloaded.auto_prune.trivial_assistant.min_age, 50);
        assert_eq!(reloaded.auto_prune.trivial_assistant.max_tokens, 40);
        assert!(!reloaded.auto_prune.anchored_assistant.enabled);
        assert_eq!(reloaded.auto_prune.anchored_assistant.radius, 42);
        assert!(reloaded.auto_prune.anchor_shield.enabled);
        assert_eq!(reloaded.auto_prune.anchor_shield.radius, 20);
        assert_eq!(reloaded.auto_prune.accumulation_threshold_tokens, 9999);
    }

    #[rstest::rstest]
    fn load_without_auto_prune_section_uses_defaults() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, "last_model = 'ollama/llama3'").expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(prefs.auto_prune.edit_read.enabled);
        assert!(prefs.auto_prune.read_edit.enabled);
        assert!(prefs.auto_prune.todo.enabled);
        assert!(prefs.auto_prune.tool_age_window.enabled);
        assert_eq!(prefs.auto_prune.tool_age_window.min_age, 150);
        assert!(prefs.auto_prune.trivial_assistant.enabled);
        assert_eq!(prefs.auto_prune.trivial_assistant.min_age, 100);
        assert_eq!(prefs.auto_prune.trivial_assistant.max_tokens, 80);
        assert!(prefs.auto_prune.anchored_assistant.enabled);
        assert_eq!(prefs.auto_prune.anchored_assistant.radius, 100);
        assert!(prefs.auto_prune.anchor_shield.enabled);
        assert_eq!(prefs.auto_prune.anchor_shield.radius, 20);
        assert_eq!(prefs.auto_prune.accumulation_threshold_tokens, 10_000);
    }

    #[rstest::rstest]
    fn load_with_anchored_assistant_radius_uses_defaults_for_anchor_shield() {
        // Given a TOML file with only anchored_assistant radius (no anchor_shield section).
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "[auto_prune.anchored_assistant]\nenabled = true\nradius = 42\nmin_age = 5\n\n[auto_prune.anchor_shield]\nenabled = true\nradius = 20\n",
        )
        .expect("write");

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then anchored_assistant still reads its own radius.
        assert!(prefs.auto_prune.anchored_assistant.enabled);
        assert_eq!(prefs.auto_prune.anchored_assistant.radius, 42);
        assert_eq!(prefs.auto_prune.anchored_assistant.min_age, 5);

        // And anchor_shield uses its own config.
        assert!(prefs.auto_prune.anchor_shield.enabled);
        assert_eq!(prefs.auto_prune.anchor_shield.radius, 20);
    }

    #[rstest::rstest]
    fn load_parses_auto_prune_todo_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, "[auto_prune.todo]\nenabled = false\n").expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert!(!prefs.auto_prune.todo.enabled);
        // edit_read should still have defaults
        assert!(prefs.auto_prune.edit_read.enabled);
        // read_edit should still have defaults
        assert!(prefs.auto_prune.read_edit.enabled);
    }

    #[rstest::rstest]
    fn default_min_age_is_50_for_new_workers() {
        // Given the three newly-min_age'd configs and the per-rule default.
        // Then their Default impls all produce min_age == 50.
        assert_eq!(AnchoredAssistantAutoPruneConfig::default().min_age, 50);
        // ConsecutiveReads min_age was raised to 80 in the defaults.
        // The other three remain 50.
        assert_eq!(ConsecutiveReadsAutoPruneConfig::default().min_age, 80);
        // RegexPruneRule has no Default impl (pattern is required), so verify
        // via the serde default function directly.
        assert_eq!(default_regex_min_age(), 50);
    }

    #[rstest::rstest]
    fn toml_roundtrip_with_renamed_fields() {
        // Given a TOML that uses the legacy aliases (`max_age_entries` and
        // `min_tail_entries`) instead of the new `min_age` field name.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "[auto_prune.trivial_assistant]\nenabled = true\nmax_age_entries = 100\nmax_tokens = 80\n\n[auto_prune.broken_edit]\nenabled = true\nmin_tail_entries = 10\n",
        )
        .expect("write");

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then the legacy keys are accepted via serde alias and populate
        // the new `min_age` field.
        assert_eq!(prefs.auto_prune.trivial_assistant.min_age, 100);
        assert_eq!(prefs.auto_prune.broken_edit.min_age, 10);
    }

    #[rstest::rstest]
    fn load_accumulation_threshold_from_toml() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            "[auto_prune]\naccumulation_threshold_tokens = 5000\n",
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert_eq!(prefs.auto_prune.accumulation_threshold_tokens, 5000);
    }

    #[rstest::rstest]
    fn accumulation_threshold_defaults_when_section_absent() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        // A sub-table exists but the top-level scalar is omitted.
        std::fs::write(&path, "[auto_prune.read_edit]\nenabled = true\n").expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert_eq!(prefs.auto_prune.accumulation_threshold_tokens, 10_000);
    }
}
