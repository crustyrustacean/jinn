//! Persistence for a stream rule's disabled state.
//!
//! [`StreamRulesStorage`] is the semantic action over the `jinn.toml`
//! storage boundary: flip one rule's `disabled` flag by name, through the
//! configuration layer's comment-preserving patch path. It exists so the
//! disable picker states what it wants ("switch `no-todos` off") instead of
//! reading the list, mutating a field, and re-serializing the whole section
//! at the call site — the lookup, the error on an unknown name, and the
//! save all belong to one place.
//!
//! The write goes through [`jinn_config::ConfigLayer`]'s `put_list`, which
//! patches the on-disk document entry-by-entry: comments, ordering, sibling
//! entries, and unknown keys all survive, and a re-armed rule loses its
//! `disabled` key entirely (the schema skips `false`). The layer is the
//! same handle an `ActionCtx` already carries, so the picker needs nothing
//! new to reach this.

use jinn_config::ConfigLayer;

use crate::schemas::StreamRuleConfig;

/// Why a rule's disabled state could not be persisted.
#[derive(Debug, PartialEq, Eq, wherror::Error)]
pub enum StreamRulesStorageError {
    /// No entry in `[[stream_rules.entry]]` carries that name.
    ///
    /// Refusing is the point of the error: silently writing nothing would
    /// report success while the armed rule kept firing.
    #[error("no stream rule is named `{name}`")]
    UnknownRule {
        /// The name the caller asked about.
        name: String,
    },
    /// The section could not be read from the document.
    #[error("stream rules could not be read from the configuration: {detail}")]
    Read {
        /// What the read failed on.
        detail: String,
    },
    /// The patched document could not be persisted.
    #[error("stream rules could not be written back: {detail}")]
    Write {
        /// What the write failed on.
        detail: String,
    },
}

/// Flipping one rule's disabled state in `jinn.toml`, by name.
///
/// Implemented for [`ConfigLayer`] — the same boundary every other
/// `jinn.toml` write crosses — so the filesystem backend and the in-memory
/// test backend are both served by the one impl, and a test asserts against
/// the exact bytes the production write produces.
pub trait StreamRulesStorage {
    /// Sets the `disabled` flag of the rule named `name` and persists the
    /// document.
    ///
    /// Finding the entry by its `name` and writing only that entry back is
    /// what keeps the rest of the user's file — per-entry comments, ordering,
    /// unknown keys, and the sibling rules — byte-identical.
    ///
    /// # Errors
    ///
    /// Returns [`StreamRulesStorageError::UnknownRule`] when no entry carries
    /// `name`, [`StreamRulesStorageError::Read`] when the section cannot be
    /// read, and [`StreamRulesStorageError::Write`] when the patched document
    /// cannot be persisted.
    fn set_stream_rule_disabled(
        &self,
        name: &str,
        disabled: bool,
    ) -> Result<(), error_stack::Report<StreamRulesStorageError>>;
}

impl StreamRulesStorage for ConfigLayer {
    fn set_stream_rule_disabled(
        &self,
        name: &str,
        disabled: bool,
    ) -> Result<(), error_stack::Report<StreamRulesStorageError>> {
        let mut rules =
            self.get_list::<StreamRuleConfig>()
                .map_err(|error| StreamRulesStorageError::Read {
                    detail: error.to_string(),
                })?;

        let Some(rule) = rules.iter_mut().find(|rule| rule.name == name) else {
            return Err(error_stack::Report::new(
                StreamRulesStorageError::UnknownRule {
                    name: name.to_owned(),
                },
            ));
        };
        rule.disabled = disabled;

        self.put_list::<StreamRuleConfig>(&rules).map_err(|error| {
            error.change_context(StreamRulesStorageError::Write {
                detail: "the patched document did not persist".to_owned(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        reason = "test assertions need expect/panic to state what went wrong"
    )]

    use std::sync::Arc;

    use jinn_common::app_info::PREFS_FILE_NAME;
    use jinn_config::{ConfigLayer, FilesystemConfigStorage, InMemoryConfigStorage};

    use super::{StreamRulesStorage, StreamRulesStorageError};
    use crate::schemas::StreamRuleConfig;

    /// A layer over the temp `jinn.toml` the test just wrote, plus the path.
    fn layer_over_file(body: &str) -> (ConfigLayer, std::path::PathBuf, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, body).expect("write");
        let layer = ConfigLayer::load(Arc::new(FilesystemConfigStorage::new(path.clone())))
            .expect("layer loads");
        (layer, path, dir)
    }

    #[rstest::rstest]
    #[test]
    fn disabling_a_rule_persists_to_a_commented_document() {
        // Given a commented jinn.toml carrying an armed rule.
        let (layer, path, _dir) = layer_over_file(
            "# keep this explanation\n[[stream_rules.entry]]\nname = \"ts-no-any\"\nconditions = [': any']\nbody = \"Use unknown.\"\n",
        );

        // When the rule is switched off through the storage boundary.
        layer
            .set_stream_rule_disabled("ts-no-any", true)
            .expect("rule disables");

        // Then the written file shows the flag beside the comment it had to
        // keep — this is the write the restart will read back.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(
            written.contains("disabled = true"),
            "disabled flag absent from the file:\n{written}"
        );
        assert!(
            written.contains("# keep this explanation"),
            "comment lost on save:\n{written}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_disabled_rule_rereads_disabled_while_its_siblings_keep_their_state() {
        // Given a jinn.toml with three armed rules.
        let (layer, _path, _dir) = layer_over_file(
            "[[stream_rules.entry]]\nname = \"alpha\"\nconditions = ['a']\nbody = \"A.\"\n\n[[stream_rules.entry]]\nname = \"beta\"\nconditions = ['b']\nbody = \"B.\"\n\n[[stream_rules.entry]]\nname = \"gamma\"\nconditions = ['g']\nbody = \"G.\"\n",
        );

        // When beta is switched off and the document is reread.
        layer
            .set_stream_rule_disabled("beta", true)
            .expect("rule disables");
        let rules = layer.get_list::<StreamRuleConfig>().expect("reread");

        // Then only beta is disabled; the roundtrip left alpha and gamma
        // armed, so a restart reads back exactly the set the toggle chose.
        assert_eq!(rules.len(), 3, "a sibling entry was lost on save");
        let disabled: Vec<_> = rules.iter().filter(|rule| rule.disabled).collect();
        assert_eq!(disabled.len(), 1, "more than one rule flipped: {rules:?}");
        assert_eq!(disabled[0].name, "beta");
    }

    #[rstest::rstest]
    #[test]
    fn disabling_leaves_sibling_entries_and_unknown_keys_untouched() {
        // Given a commented document whose rule carries a key no field
        // describes, beside a commented sibling entry.
        let (layer, path, _dir) = layer_over_file(
            "# why alpha exists\n[[stream_rules.entry]]\nname = \"alpha\"\nconditions = ['a']\nbody = \"A.\"\nseverity = \"fatal\"\n\n# why beta exists\n[[stream_rules.entry]]\nname = \"beta\"\nconditions = ['b']\nbody = \"B.\"\n",
        );

        // When alpha is switched off.
        layer
            .set_stream_rule_disabled("alpha", true)
            .expect("rule disables");

        // Then everything the toggle did not name survives the write: the
        // comments, the sibling entry, and the unknown key inside the
        // toggled entry alike.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(
            written.contains("# why alpha exists") && written.contains("# why beta exists"),
            "a comment was lost:\n{written}"
        );
        assert!(
            written.contains("severity = \"fatal\""),
            "unknown key lost:\n{written}"
        );
        assert!(
            written.contains("name = \"beta\""),
            "sibling entry lost:\n{written}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn an_unknown_name_is_an_error_and_writes_nothing() {
        // Given a commented document whose only rule is named `alpha`.
        let (layer, path, _dir) = layer_over_file(
            "# the only rule\n[[stream_rules.entry]]\nname = \"alpha\"\nconditions = ['a']\nbody = \"A.\"\n",
        );
        let before = std::fs::read_to_string(&path).expect("read");

        // When a rule that does not exist is switched off.
        let result = layer.set_stream_rule_disabled("no-such-rule", true);

        // Then the call errors naming the missing rule, and the document is
        // byte-identical — a failed toggle must not disturb the file.
        assert!(matches!(
            result,
            Err(error)
                if error.current_context()
                    == &StreamRulesStorageError::UnknownRule {
                        name: "no-such-rule".to_owned()
                    }
        ));
        let after = std::fs::read_to_string(&path).expect("read");
        assert_eq!(before, after, "an unknown name still wrote the file");
    }

    #[rstest::rstest]
    #[test]
    fn re_arming_a_rule_removes_its_disabled_key() {
        // Given a jinn.toml whose rule is written disabled.
        let (layer, path, _dir) = layer_over_file(
            "[[stream_rules.entry]]\nname = \"ts-no-any\"\nconditions = [': any']\nbody = \"Use unknown.\"\ndisabled = true\n",
        );

        // When the rule is switched back on.
        layer
            .set_stream_rule_disabled("ts-no-any", false)
            .expect("rule re-arms");

        // Then the key is gone rather than written as false, matching the
        // schema's convention that an armed rule's entry stays unchanged.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(
            !written.contains("disabled"),
            "re-arming left the key behind:\n{written}"
        );
        assert!(
            written.contains("name = \"ts-no-any\""),
            "the rule itself was lost:\n{written}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn repeating_a_no_change_toggle_does_not_churn_the_file() {
        // Given a commented document whose rule is already disabled, and a
        // first no-change write behind it — the first save normalizes a
        // hand-written entry to the schema's canonical form, which is the
        // save path's existing, pinned behavior.
        let (layer, path, _dir) = layer_over_file(
            "# kept\n[[stream_rules.entry]]\nname = \"alpha\"\nconditions = ['a']\nbody = \"A.\"\ndisabled = true\n",
        );
        layer
            .set_stream_rule_disabled("alpha", true)
            .expect("first write");
        let before = std::fs::read_to_string(&path).expect("read");

        // When the rule is disabled again.
        layer
            .set_stream_rule_disabled("alpha", true)
            .expect("second write");

        // Then the file is unchanged: the picker can repeat the action
        // without churning the bytes the user hand-maintains.
        let after = std::fs::read_to_string(&path).expect("read");
        assert_eq!(before, after, "a no-change toggle rewrote the file");
    }

    #[rstest::rstest]
    #[test]
    fn the_write_is_visible_through_the_live_layer_without_a_reload() {
        // Given an in-memory layer — the shape a route action's `ActionCtx`
        // hands the picker.
        let storage = Arc::new(InMemoryConfigStorage::new(
            "[[stream_rules.entry]]\nname = \"alpha\"\nconditions = ['a']\nbody = \"A.\"\n"
                .parse()
                .expect("parses"),
        ));
        let layer = ConfigLayer::load(storage).expect("layer loads");

        // When the rule is switched off.
        layer
            .set_stream_rule_disabled("alpha", true)
            .expect("rule disables");

        // Then a read through the same handle already sees it, so the
        // picker can confirm the new state from the layer it holds.
        let rules = layer.get_list::<StreamRuleConfig>().expect("read");
        assert!(rules[0].disabled);
    }
}
