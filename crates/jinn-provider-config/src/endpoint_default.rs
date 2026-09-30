//! Effective OpenRouter routing endpoint for a model.
//!
//! The pin lives in `providers.toml` as a `[[endpoint_defaults]]` row keyed
//! by full model id, not in session state: a routing choice is a property of
//! the model, so it applies to every session using that model — including
//! sessions created after the pin was made — and survives a restart.
//!
//! The file is read once at launch. A pin is live the moment the picker
//! writes it, but a hand-edit made mid-run needs a restart.

use super::config::{EndpointDefault, ProvidersConfig};

/// The routing endpoint configured for one model id, if any.
///
/// `model` is the full `{name}/{model}` id shown in the picker, which is
/// exactly what a `[[endpoint_defaults]]` row's `model` field carries.
/// A model with no row auto-routes.
#[must_use]
pub fn endpoint_default_for<'a>(
    config: &'a ProvidersConfig,
    model: &str,
) -> Option<&'a EndpointDefault> {
    config
        .endpoint_defaults
        .iter()
        .find(|row| row.model == model)
}

/// The routing tag configured for one model id, if any.
///
/// The shape dispatch wants: OpenRouter's routing slug, or `None` to
/// auto-route.
#[must_use]
pub fn pinned_tag(config: &ProvidersConfig, model: &str) -> Option<String> {
    endpoint_default_for(config, model).map(|row| row.tag.clone())
}

/// Whether `config` pins an endpoint for `model`.
///
/// The shape the picker wants when marking the active row: it needs a yes/no
/// about a row it already fetched, not the value of a row it may not have.
#[must_use]
pub fn has_pin(config: &ProvidersConfig, model: &str) -> bool {
    endpoint_default_for(config, model).is_some()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use std::collections::BTreeMap;

    use super::*;
    use crate::config::ProviderEntry;

    /// A config pinning one OpenRouter model, plus one model with no pin.
    fn config_with_pin() -> ProvidersConfig {
        ProvidersConfig {
            providers: BTreeMap::from([(
                "openrouter".to_owned(),
                ProviderEntry {
                    backend: "openrouter".to_owned(),
                    models: vec![
                        "anthropic/claude-sonnet-4-20250514".to_owned(),
                        "openai/gpt-oss-120b".to_owned(),
                    ],
                    base_url: None,
                    api_key_env: None,
                    requires_key: true,
                    extra_body: None,
                    context_length: None,
                    model_info: Vec::new(),
                },
            )]),
            aliases: vec![],
            default_provider: None,
            endpoint_defaults: vec![EndpointDefault {
                model: "openrouter/anthropic/claude-sonnet-4-20250514".to_owned(),
                tag: "anthropic".to_owned(),
            }],
        }
    }

    #[rstest::rstest]
    fn endpoint_default_for_matches_the_full_model_id() {
        // Given a config with a pin for one model.
        let config = config_with_pin();

        // When resolving that model.
        let row = endpoint_default_for(&config, "openrouter/anthropic/claude-sonnet-4-20250514");

        // Then its row is found.
        assert_eq!(row.map(|r| r.tag.as_str()), Some("anthropic"));
    }

    #[rstest::rstest]
    fn endpoint_default_for_returns_none_for_a_model_with_no_row() {
        // Given a config with a pin for one model.
        let config = config_with_pin();

        // When resolving a different model on the same provider.
        let row = endpoint_default_for(&config, "openrouter/openai/gpt-oss-120b");

        // Then nothing is found — that model auto-routes.
        assert!(row.is_none());
    }

    #[rstest::rstest]
    fn pinned_tag_returns_the_configured_routing_tag() {
        // Given a config with a pin for one model.
        let config = config_with_pin();

        // When resolving that model's tag.
        let tag = pinned_tag(&config, "openrouter/anthropic/claude-sonnet-4-20250514");

        // Then the configured tag comes back.
        assert_eq!(tag.as_deref(), Some("anthropic"));
    }

    #[rstest::rstest]
    fn pinned_tag_returns_none_for_an_unpinned_model() {
        // Given a config with a pin for one model.
        let config = config_with_pin();

        // When resolving another model's tag.
        let tag = pinned_tag(&config, "openrouter/openai/gpt-oss-120b");

        // Then there is nothing to force — the model auto-routes.
        assert!(tag.is_none());
    }

    #[rstest::rstest]
    fn pinned_tag_returns_none_when_no_rows_are_configured() {
        // Given a config with no endpoint defaults at all.
        let config = ProvidersConfig {
            providers: BTreeMap::new(),
            aliases: vec![],
            default_provider: None,
            endpoint_defaults: vec![],
        };

        // When resolving any model's tag.
        let tag = pinned_tag(&config, "openrouter/anthropic/claude-sonnet-4-20250514");

        // Then the model auto-routes.
        assert!(tag.is_none());
    }

    #[rstest::rstest]
    fn has_pin_reports_whether_a_row_exists() {
        // Given a config with a pin for one model.
        let config = config_with_pin();

        // Then only the pinned model reports a pin.
        assert!(has_pin(
            &config,
            "openrouter/anthropic/claude-sonnet-4-20250514"
        ));
        assert!(!has_pin(&config, "openrouter/openai/gpt-oss-120b"));
    }
}
