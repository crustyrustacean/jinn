//! The `[tools]` section — the tools slice's own configuration.
//!
//! The fallback tool timeout, the two output caps the bash tool
//! applies, and the filter a new session starts under.

use jinn_core_types::NameFilter;
use serde::{Deserialize, Serialize};

/// The `[tools]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Which tools a NEW session starts permitted or withheld.
    ///
    /// ```toml
    /// # The former blocklist: withhold these two.
    /// [tools.tool_filter]
    /// mode = "deny"
    /// names = ["bash", "write"]
    ///
    /// # An attendant that gets only what it is given, MCP included.
    /// [tools.tool_filter]
    /// mode = "allow"
    /// names = ["read", "grep", "mcp__github__*"]
    /// ```
    ///
    /// Replaces the former `disabled` key. A file still carrying `disabled`
    /// parses without it, so the filter reads as empty and permits
    /// everything — an existing blocklist silently stops applying. There is
    /// no migration; move the names under `tool_filter` yourself.
    #[serde(default, skip_serializing_if = "NameFilter::is_unconfigured")]
    pub tool_filter: NameFilter,

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
            tool_filter: NameFilter::default(),
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
    use jinn_core_types::{FilterMode, NameFilter};

    use super::ToolsConfig;

    #[rstest::rstest]
    fn a_denied_tool_is_read_from_the_filter() {
        // Given a table denying two tools.
        let table: toml::Table =
            toml::from_str("[tool_filter]\nmode = \"deny\"\nnames = [\"bash\", \"web-search\"]\n")
                .expect("test TOML parses");

        // When reading the section.
        let config = ToolsConfig::from_table(&table).expect("section deserializes");

        // Then both are withheld and the third is not.
        assert!(!config.tool_filter.permits("bash"));
        assert!(!config.tool_filter.permits("web-search"));
        assert!(config.tool_filter.permits("read"));
    }

    /// The breaking rename's silent failure, pinned where it bites: a user's
    /// `jinn.toml` blocklist stops applying and every tool comes back. There
    /// is no migration, so this is the documented outcome rather than a bug —
    /// but it is worth a test so a future change to it is deliberate.
    #[rstest::rstest]
    fn a_stale_disabled_key_reads_as_no_filter() {
        // Given a file written before the rename, still carrying `disabled`.
        let table: toml::Table = toml::from_str("disabled = [\"bash\"]").expect("test TOML parses");

        // When reading the section.
        let config = ToolsConfig::from_table(&table).expect("section deserializes");

        // Then the old key is unknown, so the tool is permitted again.
        assert_eq!(config.tool_filter, NameFilter::default());
        assert!(config.tool_filter.permits("bash"));
    }

    #[rstest::rstest]
    fn an_allow_filter_is_read_from_the_section() {
        // Given a table allowing one tool by glob.
        let table: toml::Table =
            toml::from_str("[tool_filter]\nmode = \"allow\"\nnames = [\"mcp__github__*\"]\n")
                .expect("test TOML parses");

        // When reading the section.
        let config = ToolsConfig::from_table(&table).expect("section deserializes");

        // Then only a matching tool is permitted — MCP included, which is the
        // case a blocklist could not express.
        assert_eq!(config.tool_filter.mode, FilterMode::Allow);
        assert!(config.tool_filter.permits("mcp__github__create_pr"));
        assert!(!config.tool_filter.permits("bash"));
    }

    #[rstest::rstest]
    fn a_section_with_no_filter_permits_every_tool() {
        // Given a table carrying only an unrelated key.
        let table: toml::Table =
            toml::from_str("default_timeout_secs = 60").expect("test TOML parses");

        // When reading the section.
        let config = ToolsConfig::from_table(&table).expect("section deserializes");

        // Then every tool is permitted, exactly as before filters existed.
        assert!(config.tool_filter.permits("bash"));
    }

    #[rstest::rstest]
    fn an_unconfigured_filter_writes_no_key() {
        // Given a section whose filter names nothing.
        let config = ToolsConfig::default();

        // When serializing it.
        let table = toml::Value::try_from(&config).expect("serializes");

        // Then no filter key is written, so a user's file does not gain an
        // empty table on every save.
        assert!(table.get("tool_filter").is_none());
    }
}
