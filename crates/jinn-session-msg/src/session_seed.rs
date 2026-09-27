//! Session-creation seed derived from the user's configuration.

use std::collections::{BTreeSet, HashSet};

/// Per-session defaults derived from the user's configuration at session
/// creation.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSeed {
    /// Tool names to start the session with disabled.
    pub disabled_tools: HashSet<String>,
    /// Skill names to start the session with disabled.
    pub disabled_skills: HashSet<String>,
    /// MCP servers to start the session with enabled.
    pub enabled_mcp: BTreeSet<String>,
}

impl SessionSeed {
    /// Derives a new-session seed by reading three sections from the
    /// configuration layer.
    ///
    /// Three reads rather than one aggregate read: the seed's inputs are
    /// owned by three different slices, and a kernel-side struct
    /// collecting them is what this work is removing.
    #[must_use]
    pub fn from_config(config: &jinn_config::ConfigLayer) -> Self {
        let tools = config.read::<jinn_preferences_config::schemas::ToolsConfig>();
        let skills = config.read::<jinn_preferences_config::schemas::SkillsConfig>();
        let mcp = config.read::<jinn_preferences_config::schemas::mcp::McpServersConfig>();
        Self {
            disabled_tools: tools.disabled.iter().cloned().collect(),
            disabled_skills: skills.disabled.iter().cloned().collect(),
            enabled_mcp: mcp
                .iter()
                .filter(|(_, server)| server.auto_enable)
                .map(|(name, _)| name.clone())
                .collect(),
        }
    }

    /// Returns whether the seed enables at least one MCP server.
    #[must_use]
    pub fn has_auto_enabled_mcp(&self) -> bool {
        !self.enabled_mcp.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use super::*;
    use std::sync::Arc;

    use jinn_preferences_config::schemas::mcp::McpServersConfig;
    use jinn_preferences_config::schemas::{SkillsConfig, ToolsConfig};

    /// A layer over `document`, for exercising the same read path
    /// production uses.
    fn layer(document: &str) -> jinn_config::ConfigLayer {
        let parsed = document.parse().expect("test TOML parses");
        jinn_config::ConfigLayer::load(Arc::new(jinn_config::InMemoryConfigStorage::new(parsed)))
            .expect("layer loads")
    }

    fn empty_layer() -> jinn_config::ConfigLayer {
        layer("")
    }

    #[rstest::rstest]
    fn an_unconfigured_document_leaves_everything_enabled() {
        // Given a layer with no configured sections.
        let config = empty_layer();

        // When deriving the seed.
        let seed = SessionSeed::from_config(&config);

        // Then nothing is disabled and nothing is auto-enabled.
        assert!(seed.disabled_tools.is_empty());
        assert!(seed.disabled_skills.is_empty());
        assert!(seed.enabled_mcp.is_empty());
    }

    #[rstest::rstest]
    fn disabled_tools_come_from_the_tools_section() {
        // Given a layer disabling two tools.
        let config = layer("[tools]\ndisabled = [\"bash\", \"web-search\"]\n");

        // When deriving the seed.
        let seed = SessionSeed::from_config(&config);

        // Then exactly those two are disabled.
        assert_eq!(seed.disabled_tools.len(), 2);
        assert!(seed.disabled_tools.contains("bash"));
        assert!(seed.disabled_tools.contains("web-search"));
    }

    #[rstest::rstest]
    fn disabled_skills_come_from_the_skills_section() {
        // Given a layer disabling one skill.
        let config = layer("[skills]\ndisabled = [\"phased-task-loop\"]\n");

        // When deriving the seed.
        let seed = SessionSeed::from_config(&config);

        // Then exactly that skill is disabled.
        assert!(seed.disabled_skills.contains("phased-task-loop"));
    }

    #[rstest::rstest]
    fn only_auto_enabling_servers_are_seeded_as_enabled() {
        // Given two configured servers, one with auto_enable set.
        let config = layer(
            "[mcp.alpha]\nauto_enable = true\nurl = \"http://localhost:1\"\n\
             [mcp.beta]\nauto_enable = false\nurl = \"http://localhost:2\"\n",
        );

        // When deriving the seed.
        let seed = SessionSeed::from_config(&config);

        // Then only the auto-enabling one is seeded.
        assert_eq!(seed.enabled_mcp.iter().collect::<Vec<_>>(), vec!["alpha"]);
    }

    /// The three sections belong to three different slices; the seed is
    /// the one place that reads all three, and it must not silently
    /// collapse into reading one of them.
    #[rstest::rstest]
    fn the_seed_reads_all_three_sections_independently() {
        // Given a layer configuring each section differently.
        let config = layer(
            "[tools]\ndisabled = [\"bash\"]\n\
             [skills]\ndisabled = [\"micro-task-loop\"]\n\
             [mcp.gamma]\nauto_enable = true\nurl = \"http://localhost:3\"\n",
        );

        // When deriving the seed.
        let seed = SessionSeed::from_config(&config);

        // Then each section's value lands in its own field.
        assert!(seed.disabled_tools.contains("bash"));
        assert!(seed.disabled_skills.contains("micro-task-loop"));
        assert!(seed.enabled_mcp.contains("gamma"));
        assert!(seed.has_auto_enabled_mcp());
    }

    /// A `put` through the layer must round-trip a server table, since
    /// the seed is derived from exactly what `put` wrote.
    #[rstest::rstest]
    fn a_written_server_section_reads_back_identically() {
        // Given a server written through the layer.
        let config = layer("[mcp.delta]\nauto_enable = true\n");
        let before = config.read::<McpServersConfig>();
        config
            .put::<McpServersConfig>(&before)
            .expect("layer writes the mcp section");

        // When it is read again.
        let after = config.read::<McpServersConfig>();

        // Then nothing changed.
        assert_eq!(after, before);
        assert!(after.contains_key("delta"));
    }

    #[rstest::rstest]
    fn the_tools_and_skills_defaults_are_empty() {
        // Given nothing configured.
        // Then the code defaults disable nothing.
        assert!(ToolsConfig::default().disabled.is_empty());
        assert!(SkillsConfig::default().disabled.is_empty());
    }
}
