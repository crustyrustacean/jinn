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
use jinn_provider::Backend;

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
    /// All expanded entries in order.
    resolved_list: Vec<ResolvedProvider>,
    /// Test-injected factory override.
    ///
    /// When `Some`, [`create_factory`](Self::create_factory) returns a clone of
    /// this factory for every resolved provider, bypassing backend parsing.
    /// `None` in all production paths (`from_config` leaves it unset). Used by
    /// e2e tests to serve a scripted fake through the real per-request factory
    /// resolution path in the LLM actor.
    pub(crate) factory_override: Option<Arc<dyn LlmServiceFactory>>,
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
    /// Inject a test-only factory override.
    ///
    /// When set, [`create_factory`](Self::create_factory) returns a clone of
    /// this factory for every resolved provider, bypassing backend parsing.
    /// Production code never sets this; only e2e tests use it to serve a
    /// scripted fake through the real per-request resolution path.
    #[must_use]
    pub fn with_factory_override(mut self, factory: Arc<dyn LlmServiceFactory>) -> Self {
        self.factory_override = Some(factory);
        self
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
    ) -> Result<Box<dyn LlmServiceFactory>, Report<LlmServiceError>> {
        // Test-injected override: return a clone for EVERY factory request,
        // bypassing provider resolution entirely. This is the entry point used
        // by the per-request path in the LLM actor, so checking here lets e2e
        // tests serve a scripted fake through the real resolution path without
        // registering real providers. Clones share the fake's inner
        // `Arc<Mutex<VecDeque>>` queue.
        if let Some(override_factory) = &self.factory_override {
            return Ok(Box::new(FactoryOverride(override_factory.clone())));
        }
        let resolved = self.get(id).ok_or_else(|| {
            Report::new(LlmServiceError::Config).attach(format!("unknown provider: {id}"))
        })?;
        self.create_factory_from_resolved(resolved, api_keys)
    }

    fn create_factory_from_resolved(
        &self,
        resolved: &ResolvedProvider,
        api_keys: &ApiKeys,
    ) -> Result<Box<dyn LlmServiceFactory>, Report<LlmServiceError>> {
        // Test override is handled in `create_factory` before provider resolution,
        // so this path is only reached in production (no override set).
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

        let factory = GenericLlmServiceFactory::new(
            resolved.name.clone(),
            backend,
            resolved.model.clone(),
            resolved.base_url.clone(),
            api_key,
            resolved.extra_body.clone(),
        );

        Ok(Box::new(factory))
    }
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

/// Thin delegating wrapper that lets a shared `Arc<dyn LlmServiceFactory>`
/// be returned as a fresh `Box<dyn LlmServiceFactory>` per `create_factory` call.
/// Each instance delegates to the same shared factory (and thus the same backing
/// state, e.g. the fake's `Arc<Mutex<VecDeque>>` FIFO queue).
struct FactoryOverride(Arc<dyn LlmServiceFactory>);

impl LlmServiceFactory for FactoryOverride {
    fn create(&self) -> Result<Box<dyn LlmService>, Report<LlmServiceError>> {
        self.0.create()
    }

    fn name(&self) -> &str {
        self.0.name()
    }
}

impl std::fmt::Debug for FactoryOverride {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("FactoryOverride").field(&self.0).finish()
    }
}
