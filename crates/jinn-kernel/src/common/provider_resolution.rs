//! The registry-backed [`ModelNameResolver`].
//!
//! Answers the [`ModelNameResolver`](jinn_core_types::ModelNameResolver)
//! seam over the live [`ProviderRegistryService`]: an expanded
//! `provider/model` id resolves to itself, a configured alias resolves to
//! its target expanded id, and a name that is both resolves as the expanded
//! id (registry lookup precedes alias resolution, matching the registry's
//! own precedence). Existence only — a known id with a missing API key
//! still resolves, because availability is the picker's gate, not the
//! spawn's, and keys arrive after startup for remote providers.

use std::sync::Arc;

use jinn_core_types::ModelNameResolver;
use jinn_provider_config::ProviderId;
use jinn_provider_config::ProviderRegistryService;

/// The registry-backed resolver implementation.
#[derive(Clone)]
pub struct RegistryModelNameResolver {
    registry: ProviderRegistryService,
}

impl RegistryModelNameResolver {
    /// Resolves names against `registry`.
    #[must_use]
    pub fn new(registry: ProviderRegistryService) -> Self {
        Self { registry }
    }
}

impl ModelNameResolver for RegistryModelNameResolver {
    fn resolve(&self, name: &str) -> Option<String> {
        // Expanded id first, alias second: a name that is both is the
        // expanded id, matching `ProviderRegistry` lookup precedence.
        if self
            .registry
            .get(&ProviderId::new(name.to_owned()))
            .is_some()
        {
            return Some(name.to_owned());
        }
        self.registry
            .resolve_alias(name)
            .map(|resolved| resolved.id.to_string())
    }

    fn name(&self) -> &'static str {
        "registry-model-name-resolver"
    }
}

/// Builds the shared handle the orchestrator hands to tool contexts.
#[must_use]
pub fn registry_model_name_resolver(
    registry: &ProviderRegistryService,
) -> Arc<dyn ModelNameResolver> {
    Arc::new(RegistryModelNameResolver::new(registry.clone()))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        reason = "test assertions"
    )]
    use std::collections::BTreeMap;

    use jinn_core_types::ModelNameResolver as _;
    use jinn_provider_config::{AliasEntry, ProviderEntry, ProvidersConfig};

    use super::RegistryModelNameResolver;

    /// Builds a registry with two providers and one alias:
    /// `ollama/llama3`, `openrouter/gpt-4`, and `fast` → `ollama/llama3`.
    fn resolver_with_alias() -> RegistryModelNameResolver {
        let entry = |backend: &str, models: &[&str], requires_key: bool| ProviderEntry {
            model_info: Vec::new(),
            backend: backend.to_owned(),
            models: models.iter().map(|m| (*m).to_owned()).collect(),
            base_url: None,
            api_key_env: None,
            requires_key,
            extra_body: None,
            context_length: None,
        };
        let config = ProvidersConfig {
            providers: BTreeMap::from([
                ("ollama".to_owned(), entry("ollama", &["llama3"], false)),
                (
                    "openrouter".to_owned(),
                    entry("openrouter", &["gpt-4"], true),
                ),
            ]),
            aliases: vec![AliasEntry {
                name: "fast".to_owned(),
                target: "ollama/llama3".to_owned(),
            }],
            default_provider: None,
            endpoint_defaults: Vec::new(),
        };
        RegistryModelNameResolver::new(
            jinn_provider_config::ProviderRegistry::from_config(config)
                .map(jinn_provider_config::ProviderRegistryService::new)
                .expect("valid test config"),
        )
    }

    #[rstest::rstest]
    #[test]
    fn resolve_returns_expanded_id_for_itself() {
        // Given a resolver over a registry with an expanded provider.
        let resolver = resolver_with_alias();

        // When resolving the expanded id.
        let resolved = resolver.resolve("openrouter/gpt-4");

        // Then it resolves to itself.
        assert_eq!(resolved.as_deref(), Some("openrouter/gpt-4"));
    }

    #[rstest::rstest]
    #[test]
    fn resolve_returns_alias_target_for_alias_name() {
        // Given a resolver over a registry with a configured alias.
        let resolver = resolver_with_alias();

        // When resolving the alias name.
        let resolved = resolver.resolve("fast");

        // Then it resolves to the alias's target expanded id.
        assert_eq!(resolved.as_deref(), Some("ollama/llama3"));
    }

    #[rstest::rstest]
    #[test]
    fn resolve_returns_none_for_unknown_name() {
        // Given a resolver over a populated registry.
        let resolver = resolver_with_alias();

        // When resolving a name that is neither an expanded id nor an alias.
        let resolved = resolver.resolve("no-such-model");

        // Then nothing resolves.
        assert_eq!(resolved, None);
    }

    #[rstest::rstest]
    #[test]
    fn resolve_prefers_expanded_id_when_alias_name_collides() {
        // Given a resolver whose registry has an alias shadowed by an
        // expanded id of the same name.
        let entry = ProviderEntry {
            model_info: Vec::new(),
            backend: "ollama".to_owned(),
            models: vec!["fast".to_owned()],
            base_url: None,
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        };
        let config = ProvidersConfig {
            providers: BTreeMap::from([("ollama".to_owned(), entry)]),
            aliases: vec![AliasEntry {
                name: "ollama/fast".to_owned(),
                target: "ollama/fast".to_owned(),
            }],
            default_provider: None,
            endpoint_defaults: Vec::new(),
        };
        let resolver = RegistryModelNameResolver::new(
            jinn_provider_config::ProviderRegistry::from_config(config)
                .map(jinn_provider_config::ProviderRegistryService::new)
                .expect("valid test config"),
        );

        // When resolving the colliding name.
        let resolved = resolver.resolve("ollama/fast");

        // Then the expanded id wins: the name resolves to itself, and the
        // alias is never consulted.
        assert_eq!(resolved.as_deref(), Some("ollama/fast"));
    }
}
