//! The `[skills]` section — the skills slice's own configuration.

use jinn_core_types::NameFilter;
use serde::{Deserialize, Serialize};

/// The `[skills]` section.
///
/// The filter has the same mode-plus-glob shape as `[tools]`, and covers the
/// same breaking rename: a file still carrying `disabled` parses without it
/// and every skill comes back. See [`jinn_core_types::NameFilter`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsConfig {
    /// Which skills a NEW session may load.
    #[serde(default, skip_serializing_if = "NameFilter::is_unconfigured")]
    pub skill_filter: NameFilter,
}

impl jinn_config::Configurable for SkillsConfig {
    const KEY: &'static str = "skills";
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use jinn_config::Configurable;
    use jinn_core_types::{FilterMode, NameFilter};

    use super::SkillsConfig;

    #[rstest::rstest]
    fn a_denied_skill_is_read_from_the_filter() {
        // Given a table denying one skill.
        let table: toml::Table = toml::from_str(
            r#"
            [skill_filter]
            mode = "deny"
            names = ["web-search"]
        "#,
        )
        .expect("test TOML parses");

        // When reading the section.
        let config = SkillsConfig::from_table(&table).expect("section deserializes");

        // Then the skill reads as withheld.
        assert!(!config.skill_filter.permits("web-search"));
    }

    #[rstest::rstest]
    fn an_allow_filter_is_read_from_the_section() {
        // Given a table allowing two skills by glob.
        let table: toml::Table = toml::from_str(
            r#"
            [skill_filter]
            mode = "allow"
            names = ["web-*", "dataviz"]
        "#,
        )
        .expect("test TOML parses");

        // When reading the section.
        let config = SkillsConfig::from_table(&table).expect("section deserializes");

        // Then the mode survives, and only a listed skill is permitted.
        assert_eq!(config.skill_filter.mode, FilterMode::Allow);
        assert!(config.skill_filter.permits("web-search"));
        assert!(!config.skill_filter.permits("phased-task-loop"));
    }

    #[rstest::rstest]
    fn a_section_with_no_filter_permits_every_skill() {
        // Given a table carrying only an unrelated key.
        let table: toml::Table =
            toml::from_str("default_timeout_secs = 60").expect("test TOML parses");

        // When reading the section.
        let config = SkillsConfig::from_table(&table).expect("section deserializes");

        // Then every skill is permitted, exactly as before filters existed.
        assert!(config.skill_filter.permits("web-search"));
    }

    #[rstest::rstest]
    fn a_stale_disabled_key_reads_as_no_filter() {
        // Given a file written before the rename, still carrying `disabled`.
        let table: toml::Table =
            toml::from_str("disabled = [\"web-search\"]").expect("test TOML parses");

        // When reading the section.
        let config = SkillsConfig::from_table(&table).expect("section deserializes");

        // Then the old key is unknown, so the skill comes back. This is the
        // accepted silent failure of the breaking rename: an existing
        // blocklist stops applying without anything reporting it.
        assert_eq!(config.skill_filter, NameFilter::default());
        assert!(config.skill_filter.permits("web-search"));
    }

    #[rstest::rstest]
    fn an_unconfigured_filter_is_omitted_when_serialized() {
        // Given a section whose filter names nothing.
        let config = SkillsConfig::default();

        // When serializing it.
        let table = toml::Value::try_from(&config).expect("serializes");

        // Then no filter key is written, so a user's file does not gain an
        // empty table on every save.
        assert!(table.get("skill_filter").is_none());
    }
}
