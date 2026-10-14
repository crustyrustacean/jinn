//! Persistence for a stream rule's disabled state.
//!
//! [`StreamRulesStorage`] is the semantic action over the `jinn.toml`
//! storage boundary: switch one rule off by name, through the
//! configuration layer's comment-preserving patch path. It exists so the
//! disable picker states what it wants ("switch `no-todos` off") instead
//! of reading the section, editing a list, and re-serializing it at the
//! call site — the lookup, the error on an unknown name, and the save all
//! belong to one place.
//!
//! The persisted shape is `[stream_rules] disabled_rules = ["name", ...]`
//! — the single source of truth for which rules are off. The write goes
//! through [`jinn_config::ConfigLayer::put_section_value`], which touches
//! only that one leaf of the section table: the `[[stream_rules.entry]]`
//! list beside it, the legacy `max_interrupts` key, any user stray key,
//! and every comment survive byte-identical. An array that empties is
//! removed from the file rather than written as `[]` — an armed rule
//! leaves no trace. The layer is the same handle an `ActionCtx` already
//! carries, so the picker needs nothing new to reach this.
//!
//! The unknown-name check runs against
//! [`assemble_stream_rules`](`crate::assemble_stream_rules`) — the same
//! merged set the matcher enforces and the picker lists — so a name from
//! any source (a shipped rule, a user `stream_rules.toml` sibling, a
//! jinn.toml entry, a legacy conversion) persists. The set checked is the
//! **unfiltered** one: the `disabled_rules` array is not applied here, or
//! confirming an already-disabled row — and re-arming it — would fail
//! with [`StreamRulesStorageError::UnknownRule`] precisely because the
//! rule is disabled.

use std::path::Path;

use jinn_config::ConfigLayer;

use crate::schemas::StreamRulesConfig;
use crate::stream_rules_assembly::assemble_stream_rules;

/// Why a rule's disabled state could not be persisted.
#[derive(Debug, PartialEq, Eq, wherror::Error)]
pub enum StreamRulesStorageError {
    /// No entry in `[[stream_rules.entry]]` carries that name.
    ///
    /// Refusing is the point of the error: silently writing nothing would
    /// report success while the armed rule kept firing. The check is a
    /// property of this write convenience — a hand-written array may name
    /// any rule, whatever file defined it.
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
    /// Adds `name` to, or removes it from, the `[stream_rules]
    /// disabled_rules` array and persists the document.
    ///
    /// `preferences_path` is where the run's `jinn.toml` lives; the rules a
    /// name may address are assembled from it — the sibling
    /// `stream_rules.toml` included — exactly as the matcher and the picker
    /// see the set.
    ///
    /// Writing only the array leaf — never the whole section — is what
    /// keeps the rest of the user's file byte-identical, and checking
    /// membership first is what makes a repeated no-change toggle a no-op
    /// on the bytes.
    ///
    /// # Errors
    ///
    /// Returns [`StreamRulesStorageError::UnknownRule`] when no rule from
    /// any source carries `name`, [`StreamRulesStorageError::Read`] when
    /// the section cannot be read, and [`StreamRulesStorageError::Write`]
    /// when the patched document cannot be persisted.
    fn set_stream_rule_disabled(
        &self,
        preferences_path: &Path,
        name: &str,
        disabled: bool,
    ) -> Result<(), error_stack::Report<StreamRulesStorageError>>;
}

impl StreamRulesStorage for ConfigLayer {
    fn set_stream_rule_disabled(
        &self,
        preferences_path: &Path,
        name: &str,
        disabled: bool,
    ) -> Result<(), error_stack::Report<StreamRulesStorageError>> {
        // The name check is against the merged, unfiltered set every
        // consumer sees: the matcher enforces it, the picker lists it, so
        // these are the names a toggle may address — whatever file
        // defined them.
        let rules = assemble_stream_rules(self, preferences_path);
        if !rules.iter().any(|rule| rule.name == name) {
            return Err(error_stack::Report::new(
                StreamRulesStorageError::UnknownRule {
                    name: name.to_owned(),
                },
            ));
        }

        let mut disabled_rules = self
            .get::<StreamRulesConfig>()
            .map_err(|error| StreamRulesStorageError::Read {
                detail: error.to_string(),
            })?
            .disabled_rules;
        let held = disabled_rules.iter().any(|held| held == name);
        if held == disabled {
            return Ok(());
        }
        if disabled {
            disabled_rules.push(name.to_owned());
        } else {
            disabled_rules.retain(|held| held != name);
        }

        // An emptied array removes the key: an armed set leaves no trace.
        let value = (!disabled_rules.is_empty()).then(|| {
            toml::Value::Array(
                disabled_rules
                    .iter()
                    .map(|name| toml::Value::String(name.clone()))
                    .collect(),
            )
        });
        self.put_section_value::<StreamRulesConfig>("disabled_rules", value.as_ref())
            .map_err(|error| {
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
    use crate::STREAM_RULES_FILE_NAME;
    use crate::schemas::StreamRulesConfig;

    /// A layer over the temp `jinn.toml` the test just wrote, plus the path.
    ///
    /// The path is what the storage call assembles the known-name set from —
    /// the sibling `stream_rules.toml` it names is part of that set — so
    /// every test passes its own path back in.
    fn layer_over_file(body: &str) -> (ConfigLayer, std::path::PathBuf, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, body).expect("write");
        let layer = ConfigLayer::load(Arc::new(FilesystemConfigStorage::new(path.clone())))
            .expect("layer loads");
        (layer, path, dir)
    }

    /// Writes a user `stream_rules.toml` beside `path`, the document's
    /// sibling the assembly reads.
    fn write_sibling_rules(path: &std::path::Path, body: &str) {
        let sibling = path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(STREAM_RULES_FILE_NAME);
        std::fs::write(sibling, body).expect("write sibling rules file");
    }

    /// The persisted `disabled_rules` array of `layer`.
    fn persisted_array(layer: &ConfigLayer) -> Vec<String> {
        layer
            .get::<StreamRulesConfig>()
            .expect("section reads")
            .disabled_rules
    }

    #[rstest::rstest]
    #[test]
    fn disabling_a_rule_writes_the_name_array_to_a_commented_document() {
        // Given a commented jinn.toml carrying an armed rule.
        let (layer, path, _dir) = layer_over_file(
            "# keep this explanation\n[[stream_rules.entry]]\nname = \"ts-no-any\"\nconditions = [': any']\nbody = \"Use unknown.\"\n",
        );

        // When the rule is switched off through the storage boundary.
        layer
            .set_stream_rule_disabled(&path, "ts-no-any", true)
            .expect("rule disables");

        // Then the written file carries the name under `[stream_rules]`
        // beside the comment it had to keep — this is the write the
        // restart will read back.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(
            written.contains("disabled_rules = [\"ts-no-any\"]"),
            "disabled_rules absent from the file:\n{written}"
        );
        assert!(
            written.contains("# keep this explanation"),
            "comment lost on save:\n{written}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_disabled_rule_rereads_in_the_array_while_its_siblings_stay_armed() {
        // Given a jinn.toml with three armed rules.
        let (layer, path, _dir) = layer_over_file(
            "[[stream_rules.entry]]\nname = \"alpha\"\nconditions = ['a']\nbody = \"A.\"\n\n[[stream_rules.entry]]\nname = \"beta\"\nconditions = ['b']\nbody = \"B.\"\n\n[[stream_rules.entry]]\nname = \"gamma\"\nconditions = ['g']\nbody = \"G.\"\n",
        );

        // When beta is switched off and the array is reread.
        layer
            .set_stream_rule_disabled(&path, "beta", true)
            .expect("rule disables");
        let disabled_rules = persisted_array(&layer);
        let entry_names: Vec<String> = layer
            .get_list::<crate::schemas::StreamRuleConfig>()
            .expect("reread")
            .into_iter()
            .map(|rule| rule.name)
            .collect();

        // Then only beta is named; the entries themselves are untouched, so
        // a restart reads back exactly the set the toggle chose.
        assert_eq!(entry_names.len(), 3, "a sibling entry was lost on save");
        assert_eq!(disabled_rules, vec!["beta".to_owned()]);
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
            .set_stream_rule_disabled(&path, "alpha", true)
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
        let result = layer.set_stream_rule_disabled(&path, "no-such-rule", true);

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
    fn a_rule_defined_only_in_a_sibling_rules_file_persists_by_name() {
        // Given a jinn.toml with no entries and a sibling
        // `stream_rules.toml` defining the only rule.
        let (layer, path, _dir) = layer_over_file("# nothing but a comment\n");
        write_sibling_rules(
            &path,
            "[[stream_rules.entry]]\nname = 'from-sibling'\nconditions = ['TODO']\nscopes = ['text']\nbody = 'Finish it.'\n",
        );

        // When that rule is switched off through the boundary.
        layer
            .set_stream_rule_disabled(&path, "from-sibling", true)
            .expect("rule disables");

        // Then the name lands in jinn.toml's array, beside the untouched
        // sibling: the persisted form is the name, whatever file defined
        // the rule.
        assert_eq!(persisted_array(&layer), vec!["from-sibling".to_owned()]);
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(
            written.contains("disabled_rules = [\"from-sibling\"]"),
            "the sibling-defined name must reach jinn.toml:\n{written}"
        );
        assert!(
            written.contains("# nothing but a comment"),
            "comment lost:\n{written}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_shipped_embedded_rule_is_a_known_name_without_any_file_defining_it() {
        // Given a jinn.toml whose array names a shipped rule, with no entry
        // and no sibling file anywhere.
        let (layer, path, _dir) =
            layer_over_file("[stream_rules]\ndisabled_rules = [\"no-unbounded-recursive-rm\"]\n");

        // When the shipped rule is re-armed by name.
        layer
            .set_stream_rule_disabled(&path, "no-unbounded-recursive-rm", false)
            .expect("rule re-arms");

        // Then the write succeeds against the embedded set: a name the
        // matcher enforces is a name the picker may address, entry or not.
        assert!(persisted_array(&layer).is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn a_name_in_no_source_is_an_error_and_writes_nothing() {
        // Given a jinn.toml with no rule of any source named `ghost`.
        let (layer, path, _dir) = layer_over_file("# empty\n");
        let before = std::fs::read_to_string(&path).expect("read");

        // When `ghost` is switched off.
        let result = layer.set_stream_rule_disabled(&path, "ghost", true);

        // Then it errors, and the file is byte-identical: validation runs
        // against the assembled set, and `ghost` assembles nowhere.
        assert!(matches!(
            result,
            Err(error)
                if error.current_context()
                    == &StreamRulesStorageError::UnknownRule {
                        name: "ghost".to_owned()
                    }
        ));
        let after = std::fs::read_to_string(&path).expect("read");
        assert_eq!(before, after, "an unknown name still wrote the file");
    }

    #[rstest::rstest]
    #[test]
    fn re_arming_a_rule_removes_its_name_from_the_array() {
        // Given a jinn.toml whose array already names one rule.
        let (layer, path, _dir) = layer_over_file(
            "[stream_rules]\ndisabled_rules = [\"ts-no-any\"]\n\n[[stream_rules.entry]]\nname = \"ts-no-any\"\nconditions = [': any']\nbody = \"Use unknown.\"\n",
        );

        // When the rule is switched back on.
        layer
            .set_stream_rule_disabled(&path, "ts-no-any", false)
            .expect("rule re-arms");

        // Then the name is gone from the array, which is now empty.
        assert!(persisted_array(&layer).is_empty());
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(
            written.contains("name = \"ts-no-any\""),
            "the rule itself was lost:\n{written}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn re_arming_the_last_named_rule_removes_the_disabled_rules_key() {
        // Given a jinn.toml whose array names exactly the rule to re-arm.
        let (layer, path, _dir) = layer_over_file(
            "# section note\n[stream_rules]\ndisabled_rules = [\"alpha\"]\n\n[[stream_rules.entry]]\nname = \"alpha\"\nconditions = ['a']\nbody = \"A.\"\n",
        );

        // When that rule is re-armed.
        layer
            .set_stream_rule_disabled(&path, "alpha", false)
            .expect("rule re-arms");

        // Then the key is gone entirely rather than left as `[]` — an armed
        // set leaves no trace — and the section's own comment survives.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(
            !written.contains("disabled_rules"),
            "re-arming left the key behind:\n{written}"
        );
        assert!(
            written.contains("# section note"),
            "comment lost:\n{written}"
        );
        assert!(
            written.contains("name = \"alpha\""),
            "the rule itself was lost:\n{written}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn repeating_a_no_change_toggle_does_not_churn_the_file() {
        // Given a commented document whose array already names the rule,
        // and a first no-change write behind it.
        let (layer, path, _dir) = layer_over_file(
            "# kept\n[stream_rules]\ndisabled_rules = [\"alpha\"]\n\n[[stream_rules.entry]]\nname = \"alpha\"\nconditions = ['a']\nbody = \"A.\"\n",
        );
        layer
            .set_stream_rule_disabled(&path, "alpha", true)
            .expect("first write");
        let before = std::fs::read_to_string(&path).expect("read");

        // When the rule is disabled again.
        layer
            .set_stream_rule_disabled(&path, "alpha", true)
            .expect("second write");

        // Then the file is unchanged: the picker can repeat the action
        // without churning the bytes the user hand-maintains.
        let after = std::fs::read_to_string(&path).expect("read");
        assert_eq!(before, after, "a no-change toggle rewrote the file");
    }

    #[rstest::rstest]
    #[test]
    fn an_armed_rule_keeps_a_hand_written_array_of_other_names() {
        // Given a document whose array names two rules, one not in any
        // source — a hand-written disable of a shipped rule spelled as a
        // placeholder name.
        let (layer, path, _dir) = layer_over_file(
            "[stream_rules]\ndisabled_rules = [\"no-mkfs\", \"alpha\"]\n\n[[stream_rules.entry]]\nname = \"alpha\"\nconditions = ['a']\nbody = \"A.\"\n",
        );

        // When alpha is re-armed.
        layer
            .set_stream_rule_disabled(&path, "alpha", false)
            .expect("rule re-arms");

        // Then only alpha's name went; the hand-written name for the shipped
        // rule survives untouched.
        assert_eq!(persisted_array(&layer), vec!["no-mkfs".to_owned()]);
    }

    #[rstest::rstest]
    #[test]
    fn the_write_is_visible_through_the_live_layer_without_a_reload() {
        // Given an in-memory layer — the shape a route action's `ActionCtx`
        // hands the picker — over a temp preferences path it can assemble
        // from.
        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let storage = Arc::new(InMemoryConfigStorage::new(
            "[[stream_rules.entry]]\nname = \"alpha\"\nconditions = ['a']\nbody = \"A.\"\n"
                .parse()
                .expect("parses"),
        ));
        let layer = ConfigLayer::load(storage).expect("layer loads");

        // When the rule is switched off.
        layer
            .set_stream_rule_disabled(&path, "alpha", true)
            .expect("rule disables");

        // Then a read through the same handle already sees it, so the
        // picker can confirm the new state from the layer it holds.
        assert_eq!(persisted_array(&layer), vec!["alpha".to_owned()]);
    }
}
