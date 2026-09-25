//! The `[tools]` section — the tools slice's own configuration.
//!
//! The fallback tool timeout, the two output caps the bash tool
//! applies, and the list of tools a new session starts with disabled.

use serde::{Deserialize, Serialize};

/// The `[tools]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Tools a NEW session starts with disabled, by tool name.
    ///
    /// `BTreeSet`, not `HashSet`: the patcher rewrites this array's bytes
    /// on save, and hash iteration order would reshuffle the user's list
    /// between runs.
    #[serde(default)]
    pub disabled: std::collections::BTreeSet<String>,

    /// Fallback timeout for a tool that declares none of its own, in
    /// seconds. Default: 300.
    #[serde(default = "default_timeout_secs")]
    pub default_timeout_secs: u64,

    /// Cap on the lines a tool's output contributes to the transcript.
    /// `None` = no cap.
    #[serde(default)]
    pub max_output_lines: Option<usize>,

    /// Cap on the bytes a tool's output contributes to the transcript.
    /// `None` = no cap.
    #[serde(default)]
    pub max_output_bytes: Option<usize>,
}

fn default_timeout_secs() -> u64 {
    300
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            disabled: std::collections::BTreeSet::new(),
            default_timeout_secs: default_timeout_secs(),
            max_output_lines: None,
            max_output_bytes: None,
        }
    }
}

impl jinn_config::Configurable for ToolsConfig {
    const KEY: &'static str = "tools";
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use jinn_config::Configurable;

    use super::ToolsConfig;

    #[rstest::rstest]
    fn an_absent_output_cap_reads_as_no_cap() {
        // Given a table that sets only the timeout.
        let table: toml::Table =
            toml::from_str("default_timeout_secs = 60").expect("test TOML parses");

        // When reading the section.
        let config = ToolsConfig::from_table(&table).expect("section deserializes");

        // Then the timeout is the document's and the caps stay uncapped.
        assert_eq!(config.default_timeout_secs, 60);
        assert_eq!(config.max_output_lines, None);
        assert_eq!(config.max_output_bytes, None);
    }
}
