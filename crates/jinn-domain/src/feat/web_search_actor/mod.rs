//! Web search actor — owns a [`WebSearcher`] backend and handles the
//! `web-search` tool calls.
//!
//! Subscribes to [`ExecuteWebSearch`] commands dispatched by the tool
//! orchestrator. On startup, registers the `web-search` tool definition. On
//! command, parses arguments, delegates to the [`WebSearcher`] backend
//! (currently [`DdgSearcher`]), and emits [`ToolExecutionCompleted`].
//!
//! Unlike `web-fetch`, the search backend is fixed to DuckDuckGo (via
//! [`DdgSearcher`]) — there is no backend-selection knob. Configuration is
//! limited to result count, region, and safe search.
//!
//! # Shutdown
//!
//! Stateless — no resources to release during [`Actor::on_stop`].

use serde::{Deserialize, Serialize};

/// Web search tool configuration.
///
/// Serialized as `[web_search]` in `jinn.toml`.
/// Controls the behavior of the `web-search` tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebSearchConfig {
    /// Maximum number of results to return per search. Default: `10`.
    #[serde(default = "default_max_results")]
    pub max_results: usize,
    /// DuckDuckGo region code, e.g. `"wt-wt"` (global) or `"us-en"`.
    /// Default: `"wt-wt"`.
    #[serde(default = "default_region")]
    pub region: String,
    /// Whether safe search is on. Default: `true`.
    #[serde(default = "default_safe_search")]
    pub safe_search: bool,
}

fn default_max_results() -> usize {
    10
}

fn default_region() -> String {
    "wt-wt".to_owned()
}

fn default_safe_search() -> bool {
    true
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            max_results: default_max_results(),
            region: default_region(),
            safe_search: default_safe_search(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_values() {
        // Given no input.
        // When constructing the default config.
        let config = WebSearchConfig::default();

        // Then the documented defaults are used.
        assert_eq!(config.max_results, 10);
        assert_eq!(config.region, "wt-wt");
        assert!(config.safe_search);
    }

    #[test]
    fn config_serializes_with_web_search_section() {
        // Given a default config.
        let config = WebSearchConfig::default();

        // When serializing to TOML.
        let toml = toml::to_string(&config).expect("serialize");

        // Then all three fields appear.
        assert!(toml.contains("max_results = 10"));
        assert!(toml.contains("region = \"wt-wt\""));
        assert!(toml.contains("safe_search = true"));
    }

    #[test]
    fn config_round_trips_through_toml() {
        // Given a custom config.
        let config = WebSearchConfig {
            max_results: 5,
            region: "us-en".to_owned(),
            safe_search: false,
        };

        // When serializing then deserializing.
        let toml = toml::to_string(&config).expect("serialize");
        let back: WebSearchConfig = toml::from_str(&toml).expect("deserialize");

        // Then the custom values survive.
        assert_eq!(back, config);
    }

    #[test]
    fn config_uses_defaults_when_empty() {
        // Given an empty TOML table.
        let toml = "";

        // When deserializing.
        let config: WebSearchConfig = toml::from_str(toml).expect("deserialize");

        // Then defaults are filled in.
        assert_eq!(config, WebSearchConfig::default());
    }
}
