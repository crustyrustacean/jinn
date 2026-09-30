//! The `[[attendant.entry]]` list — saved attendants, restorable by name.
//!
//! A saved attendant is a snapshot of one session's run configuration taken
//! when the user saves it from the properties popup: how it fires, what it
//! does to its context, the model and persona it runs under, and the pins
//! that carry its standing instructions. Creating from an entry reproduces
//! the configuration verbatim and inherits only what a saved file cannot
//! know — the creating session's cwd, project, and the rest of its
//! environment.
//!
//! The entry stores the *session-state* types (`AttendantBehavior`,
//! `AttendantTrigger`, `ModelSelection`, `ReasoningEffort`, `Endpoint`) via
//! serde rather than TOML-native stand-ins, so restore means "deserialize"
//! and never "translate field by field" — a divergence between the entry
//! shape and the session shape cannot compile.

use jinn_attendant_msg::{AttendantBehavior, AttendantTrigger};
use jinn_core_types::{Endpoint, ModelSelection, ReasoningEffort};
use serde::{Deserialize, Serialize};

impl jinn_config::ConfigList for AttendantEntryConfig {
    const KEY: &'static str = "attendant.entry";
    const ENTRY_KEY: &'static str = "name";
}

/// One saved attendant, defined in `jinn.toml` under `[[attendant.entry]]`.
///
/// The `name` field is the array key the patcher matches entries by, so
/// saving over an existing name replaces that entry in place — comments on
/// its siblings survive.
///
/// "Not configured" means absent: a field that is `None` (or an empty
/// disablement list) is inherited from the creating session at create time;
/// a field that is present overrides. A model serialized as the
/// no-provider placeholder counts as not configured and inherits too —
/// [`Self::configured_model`].
///
/// Every field type is `PartialEq`, so equality is derived from the struct
/// rather than compared through a serialized form. That was not always so:
/// `pins` used to hold whole chat entries, which have no `PartialEq`, and
/// [`same_entry`] compared the two documents instead.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AttendantEntryConfig {
    /// The entry's identity, and the title the created attendant gets.
    pub name: String,
    /// What a run in the attendant sees of the conversation.
    #[serde(default)]
    pub behavior: AttendantBehavior,
    /// The condition that causes an automatic re-run.
    #[serde(default)]
    pub trigger: AttendantTrigger,
    /// Whether the attendant is still being composed, and so does not run.
    ///
    /// Saved like any other run setting: an attendant saved in prep mode is
    /// a half-written one, and restoring it as a running attendant would
    /// dispatch instructions the user never finished.
    #[serde(default = "default_prep_mode")]
    pub prep_mode: bool,
    /// The text injected ahead of each run's prior report.
    #[serde(default = "default_seed_template")]
    pub seed_template: String,
    /// The model selection, absent when the saved session had no provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSelection>,
    /// The persona name, absent when the saved session ran the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_name: Option<String>,
    /// Tool names explicitly disabled, sorted and deduplicated so two
    /// saves of an unchanged attendant produce byte-identical TOML.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_tools: Vec<String>,
    /// Skill names explicitly disabled, sorted and deduplicated as above.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_skills: Vec<String>,
    /// Reasoning effort, absent when the saved session set none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Pinned OpenRouter routing endpoint, absent when none was pinned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Endpoint>,
    /// The session's pinned instructions at save time, in history order.
    ///
    /// A saved pin is a role and a text — see [`AttendantPinConfig`] for
    /// why nothing else is stored. Order is the whole point: an attendant's
    /// standing instructions are a *sequence*, so restore rebuilds them as
    /// pinned history in this order with fresh entry ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<AttendantPinConfig>,
}

/// One standing instruction a saved attendant is spawned with.
///
/// A persisted pin is only the instruction text and the role the model reads
/// it as. Everything else a chat entry carries is either derived when the
/// attendant is created or deliberately left out because a restored
/// attendant is a new session and the old values would be lies: entry ids
/// and timestamps describe the session this was saved from, and a persisted
/// token count is never recomputed — it stays stale forever.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttendantPinConfig {
    /// Whether the model reads this instruction as something the user said
    /// or something the agent produced.
    ///
    /// Required, with no `#[serde(default)]`. A user instruction and an
    /// agent result with identical text are *different messages* to the
    /// model, and nothing in the text distinguishes them. Defaulting a
    /// missing role to `user` would restore the attendant's own prior turns
    /// as facts it was told, so a malformed pin must fail loudly instead.
    pub role: AttendantPinRole,

    /// The instruction itself.
    pub text: String,
}

/// The entry kinds a saved attendant can carry.
///
/// These are exactly the kinds `entries_to_messages` turns into a message.
/// System and actor entries are excluded from context by default, and a tool
/// result is a snapshot of a file as it was — re-injected into a newly
/// spawned attendant, it asserts stale contents as current fact.
///
/// If a future change makes another kind reach the prompt, the save-side
/// filter must follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttendantPinRole {
    /// An instruction the user gave the attendant.
    User,
    /// A result the agent produced that the attendant should keep seeing.
    Assistant,
}

impl AttendantPinConfig {
    /// This instruction as a live entry, or `None` for a kind the model
    /// never reads.
    ///
    /// `None` is what keeps a tool result, system line, or actor notice out
    /// of the spawn schema — see [`AttendantPinRole`] for why.
    #[must_use]
    pub fn from_entry(entry: &jinn_core_types::ChatEntry) -> Option<Self> {
        let role = match &entry.kind {
            jinn_core_types::ChatEntryKind::User { .. } => AttendantPinRole::User,
            jinn_core_types::ChatEntryKind::Assistant(_) => AttendantPinRole::Assistant,
            _ => return None,
        };
        Some(Self {
            role,
            text: entry.text().clone(),
        })
    }

    /// This instruction as a live entry, for a fresh session to pin.
    ///
    /// The two roles construct differently: a user entry carries a
    /// display/expanded split and an assistant entry a single string. Both
    /// fields get the same text here because nothing in the tree ever makes
    /// them differ, and filling both keeps the round trip honest if that
    /// ever changes.
    ///
    /// Pin position is deliberately *not* set here — restore decides, and
    /// the reason it picks `Relative` belongs to the restore path.
    #[must_use]
    pub fn to_entry(&self) -> jinn_core_types::ChatEntry {
        match self.role {
            AttendantPinRole::User => {
                jinn_core_types::ChatEntry::user_expanded(self.text.clone(), self.text.clone())
            }
            AttendantPinRole::Assistant => jinn_core_types::ChatEntry::assistant(&self.text),
        }
    }
}

/// Serde default for the seed template, matching the session state's own
/// default so an entry written without the field seeds like a fresh
/// attendant does.
fn default_seed_template() -> String {
    jinn_attendant_msg::default_seed_template()
}

/// Prep mode's default in a config file.
///
/// True, matching the session default: an entry that does not say it is
/// composing says it is composing, the same way a session blob that omits
/// the key does.
const fn default_prep_mode() -> bool {
    true
}

impl AttendantEntryConfig {
    /// Captures a save-ready entry from a session's live configuration.
    ///
    /// `name` is the caller's decision (the session's title); everything
    /// else reads off the session state types the fields mirror. Pin
    /// entries pass through as-is — serialization drops what TOML cannot
    /// carry and restore regenerates IDs.
    #[must_use]
    pub fn from_parts(
        name: String,
        behavior: AttendantBehavior,
        trigger: AttendantTrigger,
        prep_mode: bool,
        seed_template: String,
        model: &ModelSelection,
        persona_name: &str,
        disabled_tools: &std::collections::HashSet<String>,
        disabled_skills: &std::collections::HashSet<String>,
        reasoning_effort: Option<ReasoningEffort>,
        endpoint: Option<&Endpoint>,
        pins: Vec<AttendantPinConfig>,
    ) -> Self {
        Self {
            name,
            behavior,
            trigger,
            prep_mode,
            seed_template,
            // A session with no provider configured has nothing worth
            // saving: the created attendant would refuse to dispatch, so
            // the field stays absent and inherits the parent's model.
            model: (!model.is_no_provider()).then(|| model.clone()),
            // The default persona is the default on restore too; storing
            // it would only pin the entry to today's default value.
            persona_name: (!jinn_core_types::DEFAULT_PERSONA_NAME.eq(persona_name))
                .then(|| persona_name.to_owned()),
            disabled_tools: sorted_unique(disabled_tools),
            disabled_skills: sorted_unique(disabled_skills),
            reasoning_effort,
            endpoint: endpoint.cloned(),
            pins,
        }
    }

    /// The entry's configured model, `None` when the entry carries none.
    ///
    /// The deserialized form of "no provider" is treated as absent, so a
    /// hand-written entry that names the placeholder inherits like an
    /// absent field rather than configuring the placeholder.
    #[must_use]
    pub fn configured_model(&self) -> Option<&ModelSelection> {
        self.model.as_ref().filter(|model| !model.is_no_provider())
    }
}

/// A set as a sorted, deduplicated list.
///
/// `HashSet` iteration order is run-to-run unstable; serializing one would
/// reorder the entry's lines on every save and diff the user's file
/// pointlessly. Sorting costs one allocation per save and makes the output
/// deterministic.
fn sorted_unique(names: &std::collections::HashSet<String>) -> Vec<String> {
    let mut sorted: Vec<String> = names.iter().cloned().collect();
    sorted.sort();
    sorted.dedup();
    sorted
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use std::collections::HashSet;
    use std::sync::Arc;

    use jinn_attendant_msg::{AttendantBehavior, AttendantTrigger};
    use jinn_config::{ConfigLayer, InMemoryConfigStorage};
    use jinn_core_types::{ModelSelection, NO_PROVIDER_ID};

    use super::{AttendantEntryConfig, AttendantPinConfig, AttendantPinRole};

    /// A set built from literals, for the disablement fields.
    fn set(names: impl IntoIterator<Item = String>) -> HashSet<String> {
        names.into_iter().collect()
    }

    /// A layer over a document given as a string.
    fn layer_over(body: &str) -> ConfigLayer {
        let doc = body.parse().expect("test TOML parses");
        ConfigLayer::load(Arc::new(InMemoryConfigStorage::new(doc))).expect("layer loads")
    }

    /// The plainest entry that still serializes: no model, no persona, no
    /// disablements, so a test's size or field assertions measure only what
    /// it actually set.
    fn entry_with_pins(pins: Vec<AttendantPinConfig>) -> AttendantEntryConfig {
        AttendantEntryConfig::from_parts(
            "pinned".to_owned(),
            AttendantBehavior::Reset,
            AttendantTrigger::Manual,
            false,
            jinn_attendant_msg::default_seed_template(),
            &ModelSelection::Single(NO_PROVIDER_ID.to_owned()),
            jinn_core_types::DEFAULT_PERSONA_NAME,
            &HashSet::new(),
            &HashSet::new(),
            None,
            None,
            pins,
        )
    }

    fn entry(name: &str) -> AttendantEntryConfig {
        AttendantEntryConfig::from_parts(
            name.to_owned(),
            AttendantBehavior::Reset,
            AttendantTrigger::ParentCompleted,
            false,
            "prior: <prior report>".to_owned(),
            &ModelSelection::Single("zai/glm-4.7".to_owned()),
            "reviewer",
            &set(["write".to_owned()]),
            &set(["bash".to_owned(), "edit".to_owned(), "bash".to_owned()]),
            Some(jinn_core_types::ReasoningEffort::High),
            Some(&jinn_core_types::Endpoint {
                tag: "zai".to_owned(),
                provider_name: "ZAI".to_owned(),
            }),
            Vec::new(),
        )
    }

    #[rstest::rstest]
    #[test]
    fn entries_round_trip_through_the_config_layer() {
        // Given a document carrying one full attendant entry.
        let layer = layer_over(
            r#"
            [[attendant.entry]]
            name = "reviewer"
            behavior = "reset"
            trigger = "parent_completed"
            prep_mode = false
            seed_template = "prior: <prior report>"
            model = { single = "zai/glm-4.7" }
            persona_name = "reviewer"
            disabled_tools = ["write"]
            disabled_skills = ["bash", "edit"]
            reasoning_effort = "high"

            [attendant.entry.endpoint]
            tag = "zai"
            provider_name = "ZAI"
        "#,
        );

        // When reading the list.
        let entries = layer
            .get_list::<AttendantEntryConfig>()
            .expect("list reads");

        // Then the entry reads back field for field.
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0],
            entry("reviewer"),
            "round trip lost fields: {}",
            layer.document_text()
        );
    }

    #[rstest::rstest]
    #[test]
    fn an_absent_attendant_list_reads_empty() {
        // Given a document with no attendant umbrella at all.
        let layer = layer_over("[tools]\ndefault_timeout_secs = 60");

        // When reading the list.
        let entries = layer
            .get_list::<AttendantEntryConfig>()
            .expect("list reads");

        // Then it is empty rather than an error — first-save needs no
        // special case.
        assert!(entries.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn a_minimal_entry_defaults_its_attendant_fields() {
        // Given an entry naming only what it must.
        let layer = layer_over("[[attendant.entry]]\nname = \"bare\"\n");

        // When reading the list.
        let entries = layer
            .get_list::<AttendantEntryConfig>()
            .expect("list reads");

        // Then the entry is a composing manual attendant with the shared
        // seed template and nothing configured. A bare entry does not run:
        // the same default `N` gives the session it came from.
        let e = &entries[0];
        assert_eq!(e.behavior, AttendantBehavior::Reset);
        assert_eq!(e.trigger, AttendantTrigger::Manual);
        assert!(e.prep_mode);
        assert_eq!(e.seed_template, jinn_attendant_msg::default_seed_template());
        assert_eq!(e.configured_model(), None);
        assert_eq!(e.persona_name, None);
        assert!(e.pins.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn overwriting_one_entry_preserves_sibling_comments() {
        // Given a document whose two entries carry user comments.
        let doc = r#"
            # the nightly reviewer
            [[attendant.entry]]
            name = "reviewer"
            trigger = "parent_completed"

            # runs on every parent turn
            [[attendant.entry]]
            name = "watcher"
        "#
        .parse()
        .expect("test TOML parses");
        let storage = Arc::new(InMemoryConfigStorage::new(doc));
        let layer = ConfigLayer::load(storage.clone()).expect("load");

        // When rewriting the whole list with the first entry changed.
        let mut entries = layer.get_list::<AttendantEntryConfig>().expect("read");
        entries[0].behavior = AttendantBehavior::Reset;
        layer
            .put_list::<AttendantEntryConfig>(&entries)
            .expect("list writes");

        // Then both comments survive and the edit landed.
        let text = storage.text();
        assert!(text.contains("# the nightly reviewer"), "lost:\n{text}");
        assert!(
            text.contains("# runs on every parent turn"),
            "lost:\n{text}"
        );
        let reread = layer.get_list::<AttendantEntryConfig>().expect("reread");
        assert_eq!(reread[0].behavior, AttendantBehavior::Reset);
        assert_eq!(reread[0].trigger, AttendantTrigger::ParentCompleted);
    }

    #[rstest::rstest]
    #[test]
    fn two_saves_of_an_unchanged_entry_are_byte_identical() {
        // Given a layer over an otherwise-commented document.
        let doc = "# user header\n".parse().expect("test TOML parses");
        let storage = Arc::new(InMemoryConfigStorage::new(doc));
        let layer = ConfigLayer::load(storage.clone()).expect("load");
        // With nested values present, because that is where the two save
        // paths used to disagree. An entry with no pins and no model passes
        // trivially: an empty array already rendered inline, so the fixture
        // never reached the oscillation.
        let saved = AttendantEntryConfig::from_parts(
            "reviewer".to_owned(),
            AttendantBehavior::Reset,
            AttendantTrigger::ParentCompleted,
            false,
            "prior: <prior report>".to_owned(),
            &ModelSelection::Single("zai/glm-4.7".to_owned()),
            "reviewer",
            &set(["write".to_owned()]),
            &set(["bash".to_owned()]),
            Some(jinn_core_types::ReasoningEffort::High),
            None,
            vec![
                AttendantPinConfig {
                    role: AttendantPinRole::User,
                    text: "always in context".to_owned(),
                },
                AttendantPinConfig {
                    role: AttendantPinRole::Assistant,
                    text: "the agreed verdict".to_owned(),
                },
            ],
        );

        // When saving the same entry twice (second save is an overwrite).
        layer
            .put_list::<AttendantEntryConfig>(std::slice::from_ref(&saved))
            .expect("first save");
        let once = storage.text();
        layer
            .put_list::<AttendantEntryConfig>(std::slice::from_ref(&saved))
            .expect("second save");
        let twice = storage.text();

        // Then the document did not move between saves: set-typed fields
        // serialize sorted, so a HashSet's run-to-run order cannot churn
        // the user's file.
        assert_eq!(
            once, twice,
            "document moved:\nonce:\n{once}\ntwice:\n{twice}"
        );
    }

    /// The rendered `pins` line, with spacing around `=` normalized away.
    ///
    /// The patcher preserves the whitespace the old line carried, so a file
    /// hand-edited to `pins= [...]` stays tight. Asserting on the shape
    /// rather than the spacing keeps these tests about inline-vs-header.
    fn pins_line(text: &str) -> Option<String> {
        text.lines()
            .find(|line| line.trim_start().starts_with("pins"))
            .map(|line| line.replace("pins=", "pins ="))
    }

    /// An entry with both nested-value kinds present, so a rendering
    /// assertion can cover a table and an array at once.
    fn entry_with_pins_and_model() -> AttendantEntryConfig {
        let mut e = entry_with_pins(vec![AttendantPinConfig {
            role: AttendantPinRole::User,
            text: "always in context".to_owned(),
        }]);
        e.model = Some(ModelSelection::Single("zai/glm-4.7".to_owned()));
        e
    }

    #[rstest::rstest]
    #[test]
    fn an_entries_values_all_render_as_key_value_pairs() {
        // Given an attendant carrying both kinds of nested value.
        let storage = Arc::new(InMemoryConfigStorage::new(
            "[tools]\ndefault_timeout_secs = 60\n"
                .parse()
                .expect("parses"),
        ));
        let layer = ConfigLayer::load(storage.clone()).expect("load");
        layer
            .put_list::<AttendantEntryConfig>(&[entry_with_pins_and_model()])
            .expect("write");

        // When reading the rendered document.
        let text = storage.text();

        // Then the entry is one block and every value sits on it as
        // `key = value`. A nested `[[attendant.entry.pins]]` is a *sibling*
        // of its entry in TOML's grammar, not a child — it reads as the next
        // attendant and can bind to the wrong one.
        assert!(
            text.contains("model = { single = \"zai/glm-4.7\" }"),
            "model not inline:\n{text}"
        );
        assert_eq!(
            pins_line(&text).as_deref(),
            Some("pins = [{ role = \"user\", text = \"always in context\" }]"),
            "pins not inline:\n{text}"
        );
        assert!(
            !text.contains("[attendant.entry.model]"),
            "model rendered as a sub-table:\n{text}"
        );
        assert!(
            !text.contains("[[attendant.entry.pins]]"),
            "pins rendered as a sub-list:\n{text}"
        );
        assert_eq!(
            text.matches("[[attendant.entry]]").count(),
            1,
            "expected exactly one entry block:\n{text}"
        );
    }

    #[rstest::rstest]
    #[case::no_entry_yet("")]
    #[case::already_inline(
        "[[attendant.entry]]\nname = \"pinned\"\npins = [{ role = \"user\", text = \"a\" }]\n"
    )]
    #[case::left_in_header_form(
        "[[attendant.entry]]\nname = \"pinned\"\n[[attendant.entry.pins]]\nrole = \"user\"\ntext = \"a\"\n"
    )]
    fn an_entry_renders_the_same_way_from_any_starting_shape(#[case] body: &str) {
        // Given a document in one of the shapes a save can be handed.
        let storage = Arc::new(InMemoryConfigStorage::new(
            body.parse().expect("test TOML parses"),
        ));
        let layer = ConfigLayer::load(storage.clone()).expect("load");
        let saved = entry_with_pins(vec![AttendantPinConfig {
            role: AttendantPinRole::User,
            text: "a".to_owned(),
        }]);

        // When saving it.
        layer
            .put_list::<AttendantEntryConfig>(std::slice::from_ref(&saved))
            .expect("write");

        // Then the pins render inline regardless of how the document already
        // held them — the rendered form must not depend on whether the key
        // was already present, which is what made consecutive saves disagree.
        let text = storage.text();
        assert_eq!(
            pins_line(&text).as_deref(),
            Some("pins = [{ role = \"user\", text = \"a\" }]"),
            "pins not inline from this starting shape:\n{text}"
        );
        assert!(
            !text.contains("[[attendant.entry.pins]]"),
            "pins left in header form:\n{text}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn two_entries_keep_their_own_pins() {
        // Given two attendants pinned to different instructions.
        let storage = Arc::new(InMemoryConfigStorage::new(
            "[tools]\nx = 1\n".parse().expect("parses"),
        ));
        let layer = ConfigLayer::load(storage.clone()).expect("load");
        let first = AttendantEntryConfig::from_parts(
            "first".to_owned(),
            AttendantBehavior::Reset,
            AttendantTrigger::Manual,
            false,
            jinn_attendant_msg::default_seed_template(),
            &ModelSelection::Single(NO_PROVIDER_ID.to_owned()),
            jinn_core_types::DEFAULT_PERSONA_NAME,
            &HashSet::new(),
            &HashSet::new(),
            None,
            None,
            vec![AttendantPinConfig {
                role: AttendantPinRole::User,
                text: "one".to_owned(),
            }],
        );
        let second = AttendantEntryConfig::from_parts(
            "second".to_owned(),
            AttendantBehavior::Reset,
            AttendantTrigger::Manual,
            false,
            jinn_attendant_msg::default_seed_template(),
            &ModelSelection::Single(NO_PROVIDER_ID.to_owned()),
            jinn_core_types::DEFAULT_PERSONA_NAME,
            &HashSet::new(),
            &HashSet::new(),
            None,
            None,
            vec![
                AttendantPinConfig {
                    role: AttendantPinRole::User,
                    text: "two-a".to_owned(),
                },
                AttendantPinConfig {
                    role: AttendantPinRole::User,
                    text: "two-b".to_owned(),
                },
            ],
        );

        // When saving both and reading them back.
        layer
            .put_list::<AttendantEntryConfig>(&[first, second])
            .expect("write");
        let read = layer.get_list::<AttendantEntryConfig>().expect("read");

        // Then each entry holds exactly its own pins — a nested list must
        // never bind to the neighbouring entry.
        assert_eq!(read[0].name, "first");
        assert_eq!(read[0].pins.len(), 1);
        assert_eq!(read[0].pins[0].text, "one");
        assert_eq!(read[1].name, "second");
        assert_eq!(read[1].pins.len(), 2);
        assert_eq!(read[1].pins[0].text, "two-a");
        assert_eq!(read[1].pins[1].text, "two-b");
    }

    #[rstest::rstest]
    #[test]
    fn a_comment_above_an_entry_survives_rewriting_its_pins() {
        // Given a document whose attendant entry carries a user comment.
        let storage = Arc::new(InMemoryConfigStorage::new(
            "# nightly reviewer\n[tools]\nx = 1\n"
                .parse()
                .expect("parses"),
        ));
        let layer = ConfigLayer::load(storage.clone()).expect("load");

        // When saving an attendant with pins.
        layer
            .put_list::<AttendantEntryConfig>(&[entry_with_pins(vec![AttendantPinConfig {
                role: AttendantPinRole::User,
                text: "always in context".to_owned(),
            }])])
            .expect("write");

        // Then the comment survives — inline rendering must not cost the
        // user their notes.
        assert!(
            storage.text().contains("# nightly reviewer"),
            "lost:\n{}",
            storage.text()
        );
    }

    #[rstest::rstest]
    #[test]
    fn disablement_sets_serialize_sorted_and_deduplicated() {
        // Given an entry built from a set carrying a duplicate-inserted
        // name.
        let mut tools = std::collections::HashSet::new();
        tools.insert("web_search".to_owned());
        tools.insert("edit".to_owned());
        let saved = AttendantEntryConfig::from_parts(
            "t".to_owned(),
            AttendantBehavior::Reset,
            AttendantTrigger::Manual,
            false,
            jinn_attendant_msg::default_seed_template(),
            &ModelSelection::Single(NO_PROVIDER_ID.to_owned()),
            jinn_core_types::DEFAULT_PERSONA_NAME,
            &tools,
            &std::collections::HashSet::new(),
            None,
            None,
            Vec::new(),
        );

        // When serializing to TOML.
        let value = toml::Value::try_from(&saved).expect("serializes");

        // Then the disablement list is sorted with no duplicate, and the
        // unset model/persona fields are absent rather than placeholders.
        assert_eq!(
            value
                .get("disabled_tools")
                .and_then(toml::Value::as_array)
                .map(std::vec::Vec::as_slice),
            Some(&[toml::Value::from("edit"), toml::Value::from("web_search")][..])
        );
        assert!(value.get("model").is_none());
        assert!(value.get("persona_name").is_none());
    }

    #[rstest::rstest]
    #[test]
    fn a_no_provider_model_is_never_configured() {
        // Given an entry whose model deserialized to the no-provider
        // placeholder (as a hand-written entry naming it would).
        let layer = layer_over(&format!(
            "[[attendant.entry]]\nname = \"x\"\nmodel = {{ single = \"{NO_PROVIDER_ID}\" }}\n"
        ));

        // When reading its configured model.
        let entries = layer.get_list::<AttendantEntryConfig>().expect("read");

        // Then it counts as not configured — create inherits the parent's
        // model instead of configuring the placeholder.
        assert!(entries[0].configured_model().is_none());
    }

    #[rstest::rstest]
    #[test]
    fn pinned_instructions_round_trip_through_the_document() {
        // Given a document carrying one user instruction and one the agent
        // produced.
        let saved = entry_with_pins(vec![
            AttendantPinConfig {
                role: AttendantPinRole::User,
                text: "always in context".to_owned(),
            },
            AttendantPinConfig {
                role: AttendantPinRole::Assistant,
                text: "the agreed verdict".to_owned(),
            },
        ]);
        let storage = Arc::new(InMemoryConfigStorage::new(
            "# header\n".parse().expect("parses"),
        ));
        let layer = ConfigLayer::load(storage.clone()).expect("load");
        layer
            .put_list::<AttendantEntryConfig>(std::slice::from_ref(&saved))
            .expect("write");

        // When the list is read back.
        let read = layer.get_list::<AttendantEntryConfig>().expect("read");

        // Then both pins survive, in order, with their roles intact — a
        // user instruction must not come back as an agent result.
        assert_eq!(read[0].pins, saved.pins, "document:\n{}", storage.text());
    }

    #[rstest::rstest]
    #[test]
    fn a_pin_carries_a_role_and_a_text_and_nothing_else() {
        // Given an entry carrying one instruction.
        let saved = entry_with_pins(vec![AttendantPinConfig {
            role: AttendantPinRole::User,
            text: "always in context".to_owned(),
        }]);

        // When it is serialized.
        let text = toml::to_string(&saved).expect("serializes");

        // Then no field a restored session derives — or would be lied to by
        // — reaches the file: no id, timing, token count, pin position, or
        // the display/expanded split.
        for leaked in [
            "id",
            "timing",
            "token_count",
            "pin_position",
            "display",
            "expanded",
            "context_override",
            "context_history",
        ] {
            assert!(
                !text.contains(leaked),
                "a derived field leaked into the file as {leaked}:\n{text}"
            );
        }
    }

    #[rstest::rstest]
    #[test]
    fn a_pin_without_a_role_fails_to_read_rather_than_defaulting() {
        // Given a hand-written entry whose pin names a text but no role.
        let doc = "[[attendant.entry]]\nname = \"x\"\npins = [{ text = \"hi\" }]\n";

        // When reading the list.
        let layer = layer_over(doc);

        // Then the read fails. Defaulting the role to user would restore an
        // agent result as an instruction the user gave.
        let result = layer.get_list::<AttendantEntryConfig>();

        assert!(result.is_err(), "a roleless pin was accepted");
    }

    #[rstest::rstest]
    #[case(AttendantPinRole::User, "user")]
    #[case(AttendantPinRole::Assistant, "assistant")]
    fn a_pin_role_writes_its_kind_name(#[case] role: AttendantPinRole, #[case] wire: &str) {
        // Given an entry carrying a pin of this role.
        let saved = entry_with_pins(vec![AttendantPinConfig {
            role,
            text: "hi".to_owned(),
        }]);

        // When it is serialized.
        let text = toml::to_string(&saved).expect("serializes");

        // Then the role is spelled the way `ChatEntry::kind_str` spells it,
        // so the file reads in one vocabulary.
        assert!(
            text.contains(&format!("role = \"{wire}\"")),
            "expected role = \"{wire}\":\n{text}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_long_instruction_shrinks_to_a_fraction_of_a_whole_entry() {
        // Given an instruction long enough that a whole chat entry's
        // derived fields would dominate its line.
        let instruction = "keep the module boundary intact ".repeat(40);

        // When an entry carries it as a role and text.
        let now = toml::to_string(&entry_with_pins(vec![AttendantPinConfig {
            role: AttendantPinRole::User,
            text: instruction.clone(),
        }]))
        .expect("serializes")
        .len();

        // Then it takes roughly half what the same entry took when `pins`
        // held whole chat entries. Both sides are whole entries, so the
        // shared fields cancel out and the difference is the pin's cost
        // alone. The old shape is measured, not assumed — it still
        // serializes, so this compares real bytes.
        let old = toml::to_string(&legacy_entry_with_pin(&instruction))
            .expect("serializes")
            .len();
        let ratio = f64::from(u32::try_from(now).expect("length fits")) / old as f64;
        assert!(
            (0.45..0.65).contains(&ratio),
            "expected roughly a half, got {ratio:.2} ({now} vs {old})"
        );
    }

    /// An entry shaped as `pins` used to be: whole pinned chat entries.
    ///
    /// Retained only to measure what the role-and-text schema saves. Nothing
    /// in the tree reads this shape any more — the struct exists solely so
    /// the size assertion compares against the real old bytes rather than a
    /// mirrored guess that could drift from them.
    #[derive(serde::Serialize)]
    struct LegacyEntryConfig {
        name: String,
        behavior: AttendantBehavior,
        trigger: AttendantTrigger,
        prep_mode: bool,
        seed_template: String,
        pins: Vec<jinn_core_types::ChatEntry>,
    }

    fn legacy_entry_with_pin(instruction: &str) -> LegacyEntryConfig {
        LegacyEntryConfig {
            name: "pinned".to_owned(),
            behavior: AttendantBehavior::Reset,
            trigger: AttendantTrigger::Manual,
            prep_mode: false,
            seed_template: jinn_attendant_msg::default_seed_template(),
            pins: vec![jinn_core_types::ChatEntry {
                pin_position: Some(jinn_core_types::PinPosition::Relative),
                ..jinn_core_types::ChatEntry::user(instruction)
            }],
        }
    }
}
