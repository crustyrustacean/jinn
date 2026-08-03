//! Provider registry - holds all parsed provider configs.
//!
//! [`ProviderRegistry`] is constructed from a [`ProvidersConfig`] and expands
//! each provider block's `models` list into per-model [`ResolvedProvider`]
//! entries stored in a `HashMap<ProviderId, ResolvedProvider>` for O(1) lookup.
//! Validation runs at construction time so that downstream code can trust
//! the registry contents.
//!
//! API key availability is checked against an [`ApiKeys`] store that is
//! populated once at application startup. The registry never touches the
//! environment directly.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use error_stack::{Report, ResultExt as _};
use jinn_provider::{Backend, ReasoningEffort};

use super::SampleLlmServiceFactory;
use super::api_keys::ApiKeys;
use super::config::{AliasEntry, ConfigError, ProvidersConfig};
use super::generic_factory::GenericLlmServiceFactory;
use super::provider_id::ProviderId;
use super::resolved_provider::ResolvedProvider;
use super::service::{LlmService, LlmServiceError, LlmServiceFactory};

/// Registry of configured providers.
///
/// Holds the parsed [`ProvidersConfig`] (for persistence) and the expanded
/// per-model [`ResolvedProvider`] entries (for lookup, availability, and
/// factory creation). Constructed via [`ProviderRegistry::from_config`].
#[derive(Debug, Clone)]
pub struct ProviderRegistry {
    /// The original config (for persistence - `config()`, `config_snapshot()`).
    config: ProvidersConfig,
    /// Expanded per-model entries, indexed by `ProviderId`.
    resolved_map: HashMap<ProviderId, ResolvedProvider>,
    resolved_list: Vec<ResolvedProvider>,
    /// Test-only injected factory returned by [`create_factory`](Self::create_factory)
    /// before any provider resolution, so the real per-request path exercises a
    /// scripted fake instead of erroring on the (empty in tests) registry.
    /// `None` in production. See [`with_factory_override`](Self::with_factory_override).
    factory_override: Option<FactoryOverride>,
}

impl ProviderRegistry {
    /// Creates a registry from a parsed config, validating correctness.
    ///
    /// # Validation
    ///
    /// - No duplicate provider block names.
    /// - No empty models lists.
    /// - No duplicate expanded IDs (`{name}/{model}`).
    /// - All backend strings parse via `Backend::from_str` (or are `"sample"`).
    /// - All alias targets refer to existing expanded IDs.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Validation`] if any check fails.
    pub fn from_config(config: ProvidersConfig) -> Result<Self, Report<ConfigError>> {
        // Check for duplicate provider block names.
        let mut seen_names = HashSet::new();
        for provider in &config.providers {
            if !seen_names.insert(&provider.name) {
                return Err(Report::new(ConfigError::Validation))
                    .attach(format!("duplicate provider name: {}", provider.name));
            }
        }

        // Check no empty models lists.
        for provider in &config.providers {
            if provider.models.is_empty() {
                return Err(Report::new(ConfigError::Validation)).attach(format!(
                    "provider '{}' has an empty models list",
                    provider.name
                ));
            }
        }

        // Check backend strings parse.
        for provider in &config.providers {
            if provider.backend != "sample" {
                let _: Backend = provider
                    .backend
                    .parse()
                    .change_context(ConfigError::Validation)
                    .attach(format!(
                        "invalid backend '{}' for provider '{}'",
                        provider.backend, provider.name
                    ))?;
            }
        }

        // Expand into per-model entries and check for duplicate IDs.
        let mut resolved_map: HashMap<ProviderId, ResolvedProvider> = HashMap::new();
        let mut resolved_list: Vec<ResolvedProvider> = Vec::new();

        for entry in &config.providers {
            for model in &entry.models {
                let id = ProviderId::new(format!("{}/{}", entry.name, model));
                if resolved_map.contains_key(&id) {
                    return Err(Report::new(ConfigError::Validation))
                        .attach(format!("duplicate expanded provider ID: {id}"));
                }
                let resolved = ResolvedProvider {
                    id: id.clone(),
                    name: entry.name.clone(),
                    model: model.clone(),
                    backend: entry.backend.clone(),
                    base_url: entry.base_url.clone(),
                    api_key_env: entry.api_key_env.clone(),
                    requires_key: entry.requires_key,
                    extra_body: entry.extra_body.clone(),
                    is_remote: false,
                    context_length: entry.context_length,
                };
                resolved_map.insert(id, resolved.clone());
                resolved_list.push(resolved);
            }
        }

        // Check alias targets exist in expanded set.
        for alias in &config.aliases {
            let target_id = ProviderId::new(alias.target.clone());
            if !resolved_map.contains_key(&target_id) {
                return Err(Report::new(ConfigError::Validation)).attach(format!(
                    "alias '{}' targets unknown provider '{}'",
                    alias.name, alias.target
                ));
            }
        }

        Ok(Self {
            config,
            resolved_map,
            resolved_list,
            factory_override: None,
        })
    }

    /// Inject a shared factory returned by [`create_factory`](Self::create_factory)
    /// regardless of provider id. Intended for e2e/test worlds that need the
    /// real per-request resolution path to yield a scripted fake. Production
    /// never calls this.
    #[must_use]
    pub fn with_factory_override(self, factory: Arc<dyn LlmServiceFactory>) -> Self {
        Self {
            factory_override: Some(FactoryOverride(factory)),
            ..self
        }
    }

    /// Returns the test-injected factory override, if any. Used by
    /// [`ProviderRegistryService::replace`](super::registry_service::ProviderRegistryService::replace)
    /// to carry the override across startup swaps so it survives the init actor's
    /// rebuild-from-config.
    pub(crate) fn factory_override(&self) -> Option<FactoryOverride> {
        self.factory_override.clone()
    }

    /// Restores a test-injected factory override previously captured via
    /// [`factory_override`](Self::factory_override). Used by
    /// [`ProviderRegistryService::replace`](super::registry_service::ProviderRegistryService::replace)
    /// to carry the override across startup swaps.
    pub(crate) fn set_factory_override(&mut self, factory: Option<FactoryOverride>) {
        self.factory_override = factory;
    }

    /// Returns a reference to the underlying config (for persistence).
    #[must_use]
    pub fn config(&self) -> &ProvidersConfig {
        &self.config
    }

    /// Updates the default provider in the config (for persistence on switch).
    pub fn set_default_provider(&mut self, name: Option<String>) {
        self.config.default_provider = name;
    }

    /// Merges runtime-discovered models from the model cache into the registry.
    ///
    /// For each cached model that isn't already in the registry (static entries win
    /// on collision), creates a `ResolvedProvider` with `is_remote: true` by looking
    /// up the provider block's backend, API key settings, etc.
    pub fn merge_cache(&mut self, cache: &super::ModelCache) {
        for (provider_name, models) in &cache.entries {
            let Some(entry) = self
                .config
                .providers
                .iter()
                .find(|p| p.name == *provider_name)
            else {
                continue;
            };

            for model_info in models {
                let id = ProviderId::new(format!("{}/{}", entry.name, model_info.id));
                if self.resolved_map.contains_key(&id) {
                    continue; // Static entry wins.
                }

                // Manual override from config takes precedence over API-discovered value.
                let context_length = entry.context_length.or(model_info.context_length);

                let resolved = ResolvedProvider {
                    id: id.clone(),
                    name: entry.name.clone(),
                    model: model_info.id.clone(),
                    backend: entry.backend.clone(),
                    base_url: entry.base_url.clone(),
                    api_key_env: entry.api_key_env.clone(),
                    requires_key: entry.requires_key,
                    extra_body: entry.extra_body.clone(),
                    is_remote: true,
                    context_length,
                };
                self.resolved_map.insert(id.clone(), resolved.clone());
                self.resolved_list.push(resolved);
            }
        }
    }

    /// Returns all expanded (per-model) providers.
    #[must_use]
    pub fn providers(&self) -> &[ResolvedProvider] {
        &self.resolved_list
    }

    /// Returns all configured aliases.
    #[must_use]
    pub fn aliases(&self) -> &[AliasEntry] {
        &self.config.aliases
    }

    /// Looks up a resolved provider by ID.
    #[must_use]
    pub fn get(&self, id: &ProviderId) -> Option<&ResolvedProvider> {
        self.resolved_map.get(id)
    }

    /// Resolves an alias name to its target resolved provider.
    #[must_use]
    pub fn resolve_alias<S>(&self, alias_name: S) -> Option<&ResolvedProvider>
    where
        S: AsRef<str>,
    {
        let alias_name = alias_name.as_ref();
        let alias = self.config.aliases.iter().find(|a| a.name == alias_name)?;
        self.get(&ProviderId::new(alias.target.clone()))
    }

    /// Checks whether a provider is available given the resolved API keys.
    ///
    /// Keyless providers are always available. Key-required providers are
    /// available only if their env var has a non-empty value in `api_keys`.
    #[must_use]
    pub fn is_available(&self, id: &ProviderId, api_keys: &ApiKeys) -> bool {
        let Some(resolved) = self.get(id) else {
            return false;
        };
        resolved_is_available(resolved, api_keys)
    }

    /// Returns all providers that are currently available given the resolved keys.
    #[must_use]
    pub fn available_providers(&self, api_keys: &ApiKeys) -> Vec<&ResolvedProvider> {
        self.resolved_list
            .iter()
            .filter(|p| resolved_is_available(p, api_keys))
            .collect()
    }

    /// Returns all providers that are currently unavailable (missing API key).
    #[must_use]
    pub fn unavailable_providers(&self, api_keys: &ApiKeys) -> Vec<&ResolvedProvider> {
        self.resolved_list
            .iter()
            .filter(|p| !resolved_is_available(p, api_keys))
            .collect()
    }

    /// Returns the configured default provider ID, if set and valid.
    #[must_use]
    pub fn default_provider_id(&self) -> Option<ProviderId> {
        let name = self.config.default_provider.as_ref()?;
        let id = ProviderId::new(name.clone());
        self.get(&id).is_some().then_some(id)
    }

    /// Creates an `LlmServiceFactory` for the given provider.
    ///
    /// For the special `"sample"` backend, returns [`SampleLlmServiceFactory`].
    /// For all other backends, returns [`GenericLlmServiceFactory`] with the
    /// resolved API key from `api_keys`.
    ///
    /// # Errors
    ///
    /// Returns [`LlmServiceError::Config`] if the provider is not found or
    /// the factory cannot be built. Returns [`LlmServiceError::ApiKey`] if
    /// a key-required provider is missing its key.
    pub fn create_factory(
        &self,
        id: &ProviderId,
        api_keys: &ApiKeys,
        reasoning: Option<ReasoningEffort>,
        endpoint_tag: Option<&str>,
    ) -> Result<Box<dyn LlmServiceFactory>, Report<LlmServiceError>> {
        // Test-injected override: return a clone of the scripted fake before
        // attempting provider resolution. Production never sets this (`None`).
        if let Some(override_factory) = &self.factory_override {
            return Ok(Box::new(FactoryOverride(override_factory.0.clone())));
        }
        let resolved = self.get(id).ok_or_else(|| {
            Report::new(LlmServiceError::Config).attach(format!("unknown provider: {id}"))
        })?;
        self.create_factory_from_resolved(resolved, api_keys, reasoning, endpoint_tag)
    }

    /// Creates a factory from a statically resolved provider entry.
    #[expect(clippy::unused_self, reason = "called via self from create_factory")]
    fn create_factory_from_resolved(
        &self,
        resolved: &ResolvedProvider,
        api_keys: &ApiKeys,
        reasoning: Option<ReasoningEffort>,
        endpoint_tag: Option<&str>,
    ) -> Result<Box<dyn LlmServiceFactory>, Report<LlmServiceError>> {
        if resolved.backend == "sample" {
            let factory: Box<dyn LlmServiceFactory> = Box::new(SampleLlmServiceFactory);
            return Ok(factory);
        }

        let backend: Backend = resolved
            .backend
            .parse()
            .change_context(LlmServiceError::Config)
            .attach(format!(
                "invalid backend '{}' for provider '{}'",
                resolved.backend, resolved.name
            ))?;

        // Resolve the API key.
        let api_key = if resolved.requires_key {
            let env_var = resolved.api_key_env.as_deref().unwrap_or("");
            api_keys.get(env_var).map(String::from)
        } else {
            Some("dummy-key".to_owned())
        };

        // When the backend is OpenRouter and a routing endpoint is pinned,
        // force that single upstream so every turn of the session lands on the
        // same prefix cache. `allow_fallbacks = false` is the whole point: it
        // is what keeps the cache warm. Other backends ignore `endpoint_tag`.
        let extra_body = if backend == Backend::OpenRouter {
            merge_endpoint_override(resolved.extra_body.as_ref(), endpoint_tag)
        } else {
            resolved.extra_body.clone()
        };

        let factory = GenericLlmServiceFactory::new(
            resolved.name.clone(),
            backend,
            resolved.model.clone(),
            resolved.base_url.clone(),
            api_key,
            extra_body,
            reasoning,
        );

        Ok(Box::new(factory))
    }
}


/// Merges a pinned OpenRouter routing endpoint into a provider's `extra_body`.
///
/// When `tag` is `Some`, injects `provider: { order: [tag], allow_fallbacks: false }`
/// without clobbering other keys. A `provider` key already present in `extra_body` is
/// overwritten in full (the pinned endpoint wins over any user-configured routing).
/// When `tag` is `None`, returns the existing `extra_body` untouched (auto-route).
fn merge_endpoint_override(
    existing: Option<&serde_json::Value>,
    tag: Option<&str>,
) -> Option<serde_json::Value> {
    let Some(tag) = tag else {
        return existing.cloned();
    };

    // Clone the existing map, or start fresh if there is none.
    let mut map = match existing {
        Some(serde_json::Value::Object(m)) => m.clone(),
        _ => serde_json::Map::new(),
    };

    map.insert(
        "provider".to_owned(),
        serde_json::json!({ "order": [tag], "allow_fallbacks": false }),
    );
    Some(serde_json::Value::Object(map))
}

/// Checks a single resolved provider's availability against resolved keys.
fn resolved_is_available(resolved: &ResolvedProvider, api_keys: &ApiKeys) -> bool {
    if !resolved.requires_key {
        return true;
    }
    let Some(ref env_var) = resolved.api_key_env else {
        return false;
    };
    api_keys.is_set(env_var)
}

/// Wraps a shared factory so an `Arc<dyn LlmServiceFactory>` can be returned
/// as a `Box<dyn LlmServiceFactory>` (the trait method's return type) while
/// keeping the underlying shared state (e.g. an `Arc<Mutex<...>>` queue) common
/// across all clones.
///
/// Used by the e2e harness to inject a scripted fake behind the real per-request
/// factory resolution path.
#[derive(Debug, Clone)]
pub(crate) struct FactoryOverride(Arc<dyn LlmServiceFactory>);

impl LlmServiceFactory for FactoryOverride {
    fn create(&self) -> Result<Box<dyn LlmService>, Report<LlmServiceError>> {
        self.0.create()
    }

    fn name(&self) -> &str {
        self.0.name()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::merge_endpoint_override;
    use serde_json::json;

    #[rstest::rstest]
    fn endpoint_pin_injects_provider_order_and_disables_fallbacks() {
        // Given a pinned OpenRouter endpoint tag and no existing extra_body.
        // When merging the override.
        let merged = merge_endpoint_override(None, Some("anthropic"));

        // Then the provider object pins a single upstream with no fallbacks.
        let merged = merged.expect("merged extra_body");
        let provider = merged
            .get("provider")
            .expect("provider key present");
        assert_eq!(provider["order"], json!(["anthropic"]));
        assert_eq!(provider["allow_fallbacks"], json!(false));
    }

    #[rstest::rstest]
    fn endpoint_pin_preserves_other_extra_body_keys() {
        // Given an existing extra_body with an unrelated vendor key.
        let existing = json!({ "enable_thinking": true });

        // When merging a pinned endpoint.
        let merged = merge_endpoint_override(Some(&existing), Some("azure"));

        // Then the existing key survives untouched alongside the new provider key.
        let map = merged.expect("merged extra_body");
        assert_eq!(map["enable_thinking"], json!(true));
        assert!(map.get("provider").is_some());
    }

    #[rstest::rstest]
    fn endpoint_pin_overwrites_a_prior_provider_key() {
        // Given an existing provider key from config (auto-route / fallbacks on).
        let existing = json!({ "provider": { "order": ["openai"], "allow_fallbacks": true } });

        // When merging a pinned endpoint that should win.
        let merged = merge_endpoint_override(Some(&existing), Some("anthropic"));

        // Then the pinned endpoint fully replaces the prior provider object.
        let provider = merged.expect("merged extra_body")["provider"].clone();
        assert_eq!(provider["order"], json!(["anthropic"]));
        assert_eq!(provider["allow_fallbacks"], json!(false));
    }

    #[rstest::rstest]
    fn no_pin_returns_existing_extra_body_untouched() {
        // Given an existing extra_body and no pinned tag (auto-route).
        let existing = json!({ "enable_thinking": true });

        // When merging with no tag.
        let merged = merge_endpoint_override(Some(&existing), None);

        // Then the result is the existing body unchanged with no provider key.
        assert_eq!(merged, Some(existing.clone()));
        assert!(merged.expect("merged").get("provider").is_none());
    }

    #[rstest::rstest]
    fn no_pin_and_no_existing_body_is_none() {
        // Given no existing extra_body and no pin.
        // When merging.
        // Then the result stays None (nothing to send).
        assert_eq!(merge_endpoint_override(None, None), None);
    }
}
