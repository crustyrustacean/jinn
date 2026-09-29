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
//! The entry stores the *session-state* types (`AttendantActivation`,
//! `AttendantTrigger`, `ModelSelection`, `ReasoningEffort`, `Endpoint`) via
//! serde rather than TOML-native stand-ins, so restore means "deserialize"
//! and never "translate field by field" — a divergence between the entry
//! shape and the session shape cannot compile.

use jinn_attendant_msg::{AttendantActivation, AttendantTrigger};
use jinn_core_types::{ChatEntry, Endpoint, ModelSelection, ReasoningEffort};
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
/// `PartialEq` is derived over the serialized form rather than the struct:
/// [`ChatEntry`] has no `PartialEq` of its own, and the only comparison
/// that means anything here is "the two entries write the same document" —
/// which is exactly what a round trip must preserve.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AttendantEntryConfig {
    /// The entry's identity, and the title the created attendant gets.
    pub name: String,
    /// How the attendant's context is prepared when it runs.
    #[serde(default)]
    pub activation: AttendantActivation,
    /// The condition that causes an automatic re-run.
    #[serde(default)]
    pub trigger: AttendantTrigger,
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
    /// The session's pinned entries at save time, in history order.
    ///
    /// Restored as pinned history in the same order with fresh entry IDs —
    /// replaying the saved IDs would collide with nothing, but would lie
    /// about which entries a new session's history holds. Tool-call loops
    /// are stored as their full member run, so restore appends the loop
    /// contiguously.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<ChatEntry>,
}

/// Serde default for the seed template, matching the session state's own
/// default so an entry written without the field seeds like a fresh
/// attendant does.
fn default_seed_template() -> String {
    jinn_attendant_msg::default_seed_template()
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
        activation: AttendantActivation,
        trigger: AttendantTrigger,
        seed_template: String,
        model: &ModelSelection,
        persona_name: &str,
        disabled_tools: &std::collections::HashSet<String>,
        disabled_skills: &std::collections::HashSet<String>,
        reasoning_effort: Option<ReasoningEffort>,
        endpoint: Option<&Endpoint>,
        pins: Vec<ChatEntry>,
    ) -> Self {
        Self {
            name,
            activation,
            trigger,
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

/// Whether two entries would write the same document.
///
/// Compares serialized forms: an entry's meaning is what lands in
/// `jinn.toml`, and the types it holds are not all `PartialEq`.
#[must_use]
pub fn same_entry(left: &AttendantEntryConfig, right: &AttendantEntryConfig) -> bool {
    toml::Value::try_from(left).is_ok_and(|l| toml::Value::try_from(right).is_ok_and(|r| l == r))
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

    use jinn_attendant_msg::{AttendantActivation, AttendantTrigger};
    use jinn_config::{ConfigLayer, ConfigList, InMemoryConfigStorage};
    use jinn_core_types::{ModelSelection, NO_PROVIDER_ID};

    use super::AttendantEntryConfig;

    /// A set built from literals, for the disablement fields.
    fn set(names: impl IntoIterator<Item = String>) -> HashSet<String> {
        names.into_iter().collect()
    }

    fn entry(name: &str) -> AttendantEntryConfig {
        AttendantEntryConfig::from_parts(
            name.to_owned(),
            AttendantActivation::Reset,
            AttendantTrigger::ParentCompleted,
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
        let doc = r#"
            [[attendant.entry]]
            name = "reviewer"
            activation = "reset"
            trigger = "parent_completed"
            seed_template = "prior: <prior report>"
            model = { single = "zai/glm-4.7" }
            persona_name = "reviewer"
            disabled_tools = ["write"]
            disabled_skills = ["bash", "edit"]
            reasoning_effort = "high"

            [attendant.entry.endpoint]
            tag = "zai"
            provider_name = "ZAI"
        "#
        .parse()
        .expect("test TOML parses");
        let layer = ConfigLayer::load(Arc::new(InMemoryConfigStorage::new(doc))).expect("load");

        // When reading the list.
        let entries = layer
            .get_list::<AttendantEntryConfig>()
            .expect("list reads");

        // Then the entry reads back field for field.
        assert_eq!(entries.len(), 1);
        assert!(
            super::same_entry(&entries[0], &entry("reviewer")),
            "round trip lost fields: {}",
            layer.document_text()
        );
    }

    #[rstest::rstest]
    #[test]
    fn an_absent_attendant_list_reads_empty() {
        // Given a document with no attendant umbrella at all.
        let doc = "[tools]\ndefault_timeout_secs = 60"
            .parse()
            .expect("test TOML parses");
        let layer = ConfigLayer::load(Arc::new(InMemoryConfigStorage::new(doc))).expect("load");

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
        let doc = "[[attendant.entry]]\nname = \"bare\"\n"
            .parse()
            .expect("test TOML parses");
        let layer = ConfigLayer::load(Arc::new(InMemoryConfigStorage::new(doc))).expect("load");

        // When reading the list.
        let entries = layer
            .get_list::<AttendantEntryConfig>()
            .expect("list reads");

        // Then the entry is a seed-mode manual attendant with the shared
        // seed template and nothing configured.
        let e = &entries[0];
        assert_eq!(e.activation, AttendantActivation::Seed);
        assert_eq!(e.trigger, AttendantTrigger::Manual);
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
        entries[0].activation = AttendantActivation::Reset;
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
        assert_eq!(reread[0].activation, AttendantActivation::Reset);
        assert_eq!(reread[0].trigger, AttendantTrigger::ParentCompleted);
    }

    #[rstest::rstest]
    #[test]
    fn two_saves_of_an_unchanged_entry_are_byte_identical() {
        // Given a layer over an otherwise-commented document.
        let doc = "# user header\n".parse().expect("test TOML parses");
        let storage = Arc::new(InMemoryConfigStorage::new(doc));
        let layer = ConfigLayer::load(storage.clone()).expect("load");
        let saved = entry("reviewer");

        // When saving the same entry twice (second save is an overwrite).
        layer
            .put_list::<AttendantEntryConfig>(&[saved.clone()])
            .expect("first save");
        let once = storage.text();
        layer
            .put_list::<AttendantEntryConfig>(&[saved])
            .expect("second save");
        let twice = storage.text();

        // Then the document did not move between saves: set-typed fields
        // serialize sorted, so a HashSet's run-to-run order cannot churn
        // the user's file.
        assert_eq!(once, twice);
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
            AttendantActivation::Seed,
            AttendantTrigger::Manual,
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
        let doc = format!(
            "[[attendant.entry]]\nname = \"x\"\nmodel = {{ single = \"{NO_PROVIDER_ID}\" }}\n"
        );
        let doc = doc.parse().expect("test TOML parses");
        let layer = ConfigLayer::load(Arc::new(InMemoryConfigStorage::new(doc))).expect("load");

        // When reading its configured model.
        let entries = layer.get_list::<AttendantEntryConfig>().expect("read");

        // Then it counts as not configured — create inherits the parent's
        // model instead of configuring the placeholder.
        assert!(entries[0].configured_model().is_none());
    }

    #[rstest::rstest]
    #[test]
    fn pinned_entries_round_trip_through_the_document() {
        // Given an entry carrying a pinned user entry and a pinned tool
        // result.
        let user_pin = jinn_core_types::ChatEntry {
            pin_position: Some(jinn_core_types::PinPosition::Top),
            ..jinn_core_types::ChatEntry::user("always in context")
        };
        let tool_pin = jinn_core_types::ChatEntry {
            pin_position: Some(jinn_core_types::PinPosition::Bottom),
            ..jinn_core_types::ChatEntry::tool_result(
                "call-1",
                "read_file",
                "contents".to_owned(),
                jinn_core_types::ToolResultStatus::Success,
            )
        };
        let saved = AttendantEntryConfig::from_parts(
            "pinned".to_owned(),
            AttendantActivation::Seed,
            AttendantTrigger::Manual,
            jinn_attendant_msg::default_seed_template(),
            &ModelSelection::Single(NO_PROVIDER_ID.to_owned()),
            jinn_core_types::DEFAULT_PERSONA_NAME,
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            None,
            None,
            vec![user_pin, tool_pin],
        );

        // When the entry is written and read back.
        let layer = ConfigLayer::load(Arc::new(InMemoryConfigStorage::new(
            "# header\n".parse().expect("parses"),
        )))
        .expect("load");
        layer
            .put_list::<AttendantEntryConfig>(&[saved])
            .expect("write");
        let read = layer.get_list::<AttendantEntryConfig>().expect("read");

        // Then both pins survive with their positions and content.
        let pins = &read[0].pins;
        assert_eq!(pins.len(), 2);
        assert_eq!(
            pins[0].pin_position,
            Some(jinn_core_types::PinPosition::Top)
        );
        assert_eq!(
            pins[1].pin_position,
            Some(jinn_core_types::PinPosition::Bottom)
        );
        assert_eq!(
            pins[0].text(),
            "always in context",
            "pin content must survive: {}",
            layer.document_text()
        );
    }
}
