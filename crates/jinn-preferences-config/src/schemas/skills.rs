//! The `[skills]` section — the skills slice's own configuration.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The `[skills]` section.
///
/// `BTreeSet` rather than `HashSet` deliberately: the patcher rewrites
/// the array's bytes, and a hash iteration order would reshuffle the
/// user's list between saves.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsConfig {
    /// Skills the user has turned off. Absent entries are not installed.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub disabled: BTreeSet<String>,
}

impl jinn_config::Configurable for SkillsConfig {
    const KEY: &'static str = "skills";
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use jinn_config::Configurable;

    use super::SkillsConfig;

    #[rstest::rstest]
    fn a_disabled_skill_is_read_from_the_list() {
        // Given a table listing one disabled skill.
        let table: toml::Table =
            toml::from_str("disabled = [\"web-search\"]").expect("test TOML parses");

        // When reading the section.
        let config = SkillsConfig::from_table(&table).expect("section deserializes");

        // Then the skill reads as disabled.
        assert!(config.disabled.contains("web-search"));
    }
}
