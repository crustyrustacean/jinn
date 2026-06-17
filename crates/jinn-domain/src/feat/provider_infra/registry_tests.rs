#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use super::registry::*;
use crate::feat::provider_infra::api_keys::ApiKeys;
use crate::feat::provider_infra::config::{AliasEntry, ProviderEntry, ProvidersConfig};
use crate::feat::provider_infra::provider_id::ProviderId;

fn make_config(
    providers: Vec<ProviderEntry>,
    aliases: Vec<AliasEntry>,
    default_provider: Option<&str>,
) -> ProvidersConfig {
    ProvidersConfig {
        providers,
        aliases,
        default_provider: default_provider.map(String::from),
    }
}

fn ollama_entry() -> ProviderEntry {
    ProviderEntry {
        name: "ollama".to_owned(),
        backend: "ollama".to_owned(),
        models: vec!["llama3".to_owned()],
        base_url: Some("http://localhost:11434".to_owned()),
        api_key_env: None,
        requires_key: false,
        extra_body: None,
        context_length: None,
    }
}

fn openrouter_entry() -> ProviderEntry {
    ProviderEntry {
        name: "openrouter".to_owned(),
        backend: "openrouter".to_owned(),
        models: vec!["gpt-4".to_owned()],
        base_url: None,
        api_key_env: Some("OPENROUTER_API_KEY".to_owned()),
        requires_key: true,
        extra_body: None,
        context_length: None,
    }
}

#[rstest::rstest]
fn rejects_duplicate_provider_names() {
    // Given a config with duplicate provider names.
    let config = make_config(vec![ollama_entry(), ollama_entry()], vec![], None);

    // When building the registry.
    let result = ProviderRegistry::from_config(config);

    // Then it fails with a validation error.
    assert!(result.is_err());
}

#[rstest::rstest]
fn rejects_unknown_alias_target() {
    // Given a config with an alias pointing to a non-existent expanded ID.
    let config = make_config(
        vec![ollama_entry()],
        vec![AliasEntry {
            name: "fast".to_owned(),
            target: "nonexistent/model".to_owned(),
        }],
        None,
    );

    // When building the registry.
    let result = ProviderRegistry::from_config(config);

    // Then it fails with a validation error.
    assert!(result.is_err());
}

#[rstest::rstest]
fn rejects_invalid_backend_string() {
    // Given a config with an invalid backend string.
    let config = make_config(
        vec![ProviderEntry {
            name: "bad".to_owned(),
            backend: "not-a-real-backend".to_owned(),
            models: vec!["x".to_owned()],
            base_url: None,
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        vec![],
        None,
    );

    // When building the registry.
    let result = ProviderRegistry::from_config(config);

    // Then it fails with a validation error.
    assert!(result.is_err());
}

#[rstest::rstest]
fn rejects_empty_models_list() {
    // Given a config with a provider that has an empty models list.
    let config = make_config(
        vec![ProviderEntry {
            name: "empty".to_owned(),
            backend: "ollama".to_owned(),
            models: vec![],
            base_url: None,
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        vec![],
        None,
    );

    // When building the registry.
    let result = ProviderRegistry::from_config(config);

    // Then it fails with a validation error.
    assert!(result.is_err());
}

#[rstest::rstest]
fn registry_has_two_entries() {
    // Given a config with one provider that has two models.
    let config = make_config(
        vec![ProviderEntry {
            name: "ollama".to_owned(),
            backend: "ollama".to_owned(),
            models: vec!["llama3".to_owned(), "mistral".to_owned()],
            base_url: None,
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        vec![],
        None,
    );

    // When building the registry.
    let registry = ProviderRegistry::from_config(config).expect("registry");

    // Then two resolved entries exist.
    let providers = registry.providers();
    assert_eq!(providers.len(), 2);
}

#[rstest::rstest]
fn entries_have_correct_ids() {
    // Given a config with one provider that has two models.
    let config = make_config(
        vec![ProviderEntry {
            name: "ollama".to_owned(),
            backend: "ollama".to_owned(),
            models: vec!["llama3".to_owned(), "mistral".to_owned()],
            base_url: None,
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        vec![],
        None,
    );

    // When building the registry.
    let registry = ProviderRegistry::from_config(config).expect("registry");

    // Then each has the correct expanded ID.
    let providers = registry.providers();
    assert_eq!(providers[0].id.as_str(), "ollama/llama3");
    assert_eq!(providers[0].name, "ollama");
    assert_eq!(providers[0].model, "llama3");

    assert_eq!(providers[1].id.as_str(), "ollama/mistral");
    assert_eq!(providers[1].name, "ollama");
    assert_eq!(providers[1].model, "mistral");
}

#[rstest::rstest]
fn entries_are_individually_lookupable() {
    // Given a config with one provider that has two models.
    let config = make_config(
        vec![ProviderEntry {
            name: "ollama".to_owned(),
            backend: "ollama".to_owned(),
            models: vec!["llama3".to_owned(), "mistral".to_owned()],
            base_url: None,
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        vec![],
        None,
    );

    // When building the registry.
    let registry = ProviderRegistry::from_config(config).expect("registry");

    // Then both are individually look-up-able.
    assert!(
        registry
            .get(&ProviderId::new("ollama/llama3".to_owned()))
            .is_some()
    );
    assert!(
        registry
            .get(&ProviderId::new("ollama/mistral".to_owned()))
            .is_some()
    );
}

#[rstest::rstest]
fn is_available_returns_true_for_keyless_provider() {
    // Given a registry with a keyless provider (Ollama).
    let config = make_config(vec![ollama_entry()], vec![], None);
    let registry = ProviderRegistry::from_config(config).expect("registry");
    let api_keys = ApiKeys::new();

    // When checking availability.
    // Then the keyless provider is always available.
    assert!(registry.is_available(&ProviderId::new("ollama/llama3".to_owned()), &api_keys));
}

#[rstest::rstest]
fn is_available_returns_true_when_key_resolved() {
    // Given a registry with a key-required provider and a resolved key.
    let config = make_config(vec![openrouter_entry()], vec![], None);
    let registry = ProviderRegistry::from_config(config).expect("registry");
    let mut api_keys = ApiKeys::new();
    api_keys.insert("OPENROUTER_API_KEY".to_owned(), "sk-test-value".to_owned());

    // When checking availability.
    assert!(registry.is_available(&ProviderId::new("openrouter/gpt-4".to_owned()), &api_keys));
}

#[rstest::rstest]
fn is_available_returns_false_when_key_missing() {
    // Given a registry with a key-required provider and no resolved key.
    let config = make_config(vec![openrouter_entry()], vec![], None);
    let registry = ProviderRegistry::from_config(config).expect("registry");
    let api_keys = ApiKeys::new();

    // When checking availability.
    assert!(!registry.is_available(&ProviderId::new("openrouter/gpt-4".to_owned()), &api_keys));
}

#[rstest::rstest]
fn available_providers_filters_correctly() {
    // Given a registry with one keyless and one key-required provider (no key).
    let config = make_config(vec![ollama_entry(), openrouter_entry()], vec![], None);
    let registry = ProviderRegistry::from_config(config).expect("registry");
    let api_keys = ApiKeys::new();

    // When asking for available providers.
    let available = registry.available_providers(&api_keys);

    // Then only the keyless provider is available.
    assert_eq!(available.len(), 1);
    assert_eq!(available[0].name, "ollama");
    assert_eq!(available[0].model, "llama3");
}

#[rstest::rstest]
fn resolve_alias_finds_target() {
    // Given a registry with an alias pointing to a full expanded ID.
    let config = make_config(
        vec![ollama_entry()],
        vec![AliasEntry {
            name: "fast".to_owned(),
            target: "ollama/llama3".to_owned(),
        }],
        None,
    );
    let registry = ProviderRegistry::from_config(config).expect("registry");

    // When resolving the alias.
    let resolved = registry.resolve_alias("fast");

    // Then the target resolved provider is returned.
    assert!(resolved.is_some());
    let resolved = resolved.unwrap();
    assert_eq!(resolved.name, "ollama");
    assert_eq!(resolved.model, "llama3");
}

#[rstest::rstest]
fn resolve_alias_returns_none_for_unknown() {
    // Given a registry with no matching alias.
    let config = make_config(vec![ollama_entry()], vec![], None);
    let registry = ProviderRegistry::from_config(config).expect("registry");

    // When resolving a nonexistent alias.
    assert!(registry.resolve_alias("missing").is_none());
}

#[rstest::rstest]
fn create_factory_succeeds_for_sample_backend() {
    // Given a registry with a sample provider.
    let config = make_config(
        vec![ProviderEntry {
            name: "sample".to_owned(),
            backend: "sample".to_owned(),
            models: vec!["sample".to_owned()],
            base_url: None,
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        vec![],
        None,
    );
    let registry = ProviderRegistry::from_config(config).expect("registry");
    let api_keys = ApiKeys::new();

    // When creating a factory.
    let factory = registry.create_factory(&ProviderId::new("sample/sample".to_owned()), &api_keys);

    // Then it succeeds and returns a factory named "Sample".
    assert!(factory.is_ok());
    assert_eq!(factory.unwrap().name(), "Sample");
}

#[rstest::rstest]
fn create_factory_succeeds_for_keyless_openai_backend() {
    // Given a registry with an LMStudio-like provider (OpenAI backend, no key required).
    let config = make_config(
        vec![ProviderEntry {
            name: "lmstudio".to_owned(),
            backend: "openai".to_owned(),
            models: vec!["local-model".to_owned()],
            base_url: Some("http://localhost:1234/v1".to_owned()),
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        vec![],
        None,
    );
    let registry = ProviderRegistry::from_config(config).expect("registry");
    let api_keys = ApiKeys::new();

    // When creating a factory with no API keys resolved.
    // Note: create() only builds the provider struct - no network request is made.
    let factory = registry.create_factory(
        &ProviderId::new("lmstudio/local-model".to_owned()),
        &api_keys,
    );

    // Then it succeeds (dummy key is substituted for keyless providers).
    assert!(factory.is_ok());
}

#[rstest::rstest]
fn default_provider_id_returns_configured() {
    // Given a config with a default provider.
    let config = make_config(vec![ollama_entry()], vec![], Some("ollama/llama3"));
    let registry = ProviderRegistry::from_config(config).expect("registry");

    // When asking for the default.
    let id = registry.default_provider_id();

    // Then the configured ID is returned.
    assert_eq!(id.as_ref().map(ProviderId::as_str), Some("ollama/llama3"));
}

#[rstest::rstest]
fn default_provider_id_returns_none_when_unset() {
    // Given a config with no default provider.
    let config = make_config(vec![ollama_entry()], vec![], None);
    let registry = ProviderRegistry::from_config(config).expect("registry");

    // When asking for the default.
    assert!(registry.default_provider_id().is_none());
}

#[rstest::rstest]
fn default_provider_id_returns_none_for_invalid_target() {
    // Given a config with a default that doesn't match any expanded ID.
    let config = make_config(vec![ollama_entry()], vec![], Some("ollama"));
    let registry = ProviderRegistry::from_config(config).expect("registry");

    // When asking for the default.
    // Then None is returned (old-style name no longer valid).
    assert!(registry.default_provider_id().is_none());
}

#[rstest::rstest]
fn set_default_provider_updates_config() {
    // Given a registry with a provider.
    let config = make_config(vec![ollama_entry()], vec![], None);
    let mut registry = ProviderRegistry::from_config(config).expect("registry");

    // When setting the default provider.
    registry.set_default_provider(Some("ollama/llama3".to_owned()));

    // Then default_provider_id returns the updated value.
    let id = registry.default_provider_id();
    assert_eq!(id.as_ref().map(ProviderId::as_str), Some("ollama/llama3"));
}

#[rstest::rstest]
fn set_default_provider_clears_when_none() {
    // Given a registry with a default provider.
    let config = make_config(vec![ollama_entry()], vec![], Some("ollama/llama3"));
    let mut registry = ProviderRegistry::from_config(config).expect("registry");
    assert!(registry.default_provider_id().is_some());

    // When clearing the default provider.
    registry.set_default_provider(None);

    // Then default_provider_id returns None.
    assert!(registry.default_provider_id().is_none());
}

#[rstest::rstest]
fn config_accessor_returns_config() {
    // Given a registry with providers.
    let config = make_config(
        vec![ollama_entry(), openrouter_entry()],
        vec![],
        Some("ollama/llama3"),
    );
    let registry = ProviderRegistry::from_config(config).expect("registry");

    // When accessing the config.
    let config = registry.config();

    // Then it has the expected provider blocks and default.
    assert_eq!(config.providers.len(), 2);
    assert_eq!(config.default_provider.as_deref(), Some("ollama/llama3"));
}

#[rstest::rstest]
fn create_factory_for_model_succeeds_for_known_provider() {
    // Given a registry with ollama.
    let config = make_config(vec![ollama_entry()], vec![], None);
    let mut registry = ProviderRegistry::from_config(config).expect("registry");
    let api_keys = ApiKeys::new();

    // And a cache with a remote model.
    let mut cache_entries = std::collections::HashMap::new();
    cache_entries.insert(
        "ollama".to_owned(),
        vec![crate::feat::provider_infra::ModelInfo {
            id: "mistral".to_owned(),
            context_length: None,
        }],
    );
    let cache = crate::feat::provider_infra::ModelCache {
        entries: cache_entries,
        last_updated_at: None,
    };
    registry.merge_cache(&cache);

    // When creating a factory for the remote model.
    let factory = registry.create_factory(&ProviderId::new("ollama/mistral".to_owned()), &api_keys);

    // Then it succeeds.
    assert!(factory.is_ok());
    assert_eq!(factory.unwrap().name(), "ollama");
}

#[rstest::rstest]
fn create_factory_fails_for_unknown_provider_after_merge() {
    // Given a registry with ollama.
    let config = make_config(vec![ollama_entry()], vec![], None);
    let registry = ProviderRegistry::from_config(config).expect("registry");
    let api_keys = ApiKeys::new();

    // When creating a factory for an unknown provider (no cache merged).
    let factory = registry.create_factory(&ProviderId::new("unknown/model".to_owned()), &api_keys);

    // Then it fails.
    assert!(factory.is_err());
}

#[rstest::rstest]
fn create_factory_succeeds_for_merged_remote_model() {
    // Given a registry with an LMStudio-like provider that has "local-model" as a
    // static placeholder, and a cache with a runtime-discovered model.
    let config = make_config(
        vec![ProviderEntry {
            name: "lmstudio".to_owned(),
            backend: "openai".to_owned(),
            models: vec!["local-model".to_owned()],
            base_url: Some("http://localhost:1234/v1".to_owned()),
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        vec![],
        None,
    );
    let mut registry = ProviderRegistry::from_config(config).expect("registry");
    let api_keys = ApiKeys::new();

    let mut cache_entries = std::collections::HashMap::new();
    cache_entries.insert(
        "lmstudio".to_owned(),
        vec![crate::feat::provider_infra::ModelInfo {
            id: "my-real-model".to_owned(),
            context_length: None,
        }],
    );
    let cache = crate::feat::provider_infra::ModelCache {
        entries: cache_entries,
        last_updated_at: None,
    };
    registry.merge_cache(&cache);

    // When creating a factory for the merged remote model.
    let factory = registry.create_factory(
        &ProviderId::new("lmstudio/my-real-model".to_owned()),
        &api_keys,
    );

    // Then it succeeds (merged into registry).
    assert!(factory.is_ok());
    assert_eq!(factory.unwrap().name(), "lmstudio");
}

#[rstest::rstest]
fn create_factory_for_static_model_still_works() {
    // Given a registry with a keyless provider.
    let config = make_config(vec![ollama_entry()], vec![], None);
    let registry = ProviderRegistry::from_config(config).expect("registry");
    let api_keys = ApiKeys::new();

    // When creating a factory for the statically configured model.
    let factory = registry.create_factory(&ProviderId::new("ollama/llama3".to_owned()), &api_keys);

    // Then it succeeds (unchanged behavior).
    assert!(factory.is_ok());
    assert_eq!(factory.unwrap().name(), "ollama");
}

#[rstest::rstest]
fn create_factory_succeeds_for_merged_model_with_slashes() {
    // Given a registry with an OpenRouter-like provider whose models contain slashes,
    // and a cache with a remote model.
    let config = make_config(
        vec![ProviderEntry {
            name: "openrouter".to_owned(),
            backend: "openrouter".to_owned(),
            models: vec!["openai/gpt-4".to_owned()],
            base_url: None,
            api_key_env: Some("OPENROUTER_API_KEY".to_owned()),
            requires_key: true,
            extra_body: None,
            context_length: None,
        }],
        vec![],
        None,
    );
    let mut registry = ProviderRegistry::from_config(config).expect("registry");
    let mut api_keys = ApiKeys::new();
    api_keys.insert("OPENROUTER_API_KEY".to_owned(), "sk-test".to_owned());

    let mut cache_entries = std::collections::HashMap::new();
    cache_entries.insert(
        "openrouter".to_owned(),
        vec![crate::feat::provider_infra::ModelInfo {
            id: "anthropic/claude-sonnet-4".to_owned(),
            context_length: None,
        }],
    );
    let cache = crate::feat::provider_infra::ModelCache {
        entries: cache_entries,
        last_updated_at: None,
    };
    registry.merge_cache(&cache);

    // When creating a factory for the merged remote model with a slash in the name.
    let factory = registry.create_factory(
        &ProviderId::new("openrouter/anthropic/claude-sonnet-4".to_owned()),
        &api_keys,
    );

    // Then it succeeds.
    assert!(factory.is_ok());
    assert_eq!(factory.unwrap().name(), "openrouter");
}

#[rstest::rstest]
fn registry_propagates_extra_body_to_resolved_provider() {
    // Given a config with extra_body.
    let config = make_config(
        vec![ProviderEntry {
            name: "zai".to_owned(),
            backend: "ollama".to_owned(), // Use ollama to avoid key requirement
            models: vec!["glm-5.1".to_owned()],
            base_url: None,
            api_key_env: None,
            requires_key: false,
            extra_body: Some(serde_json::json!({"enable_thinking": true, "tool_stream": true})),
            context_length: None,
        }],
        vec![],
        None,
    );

    // When building the registry.
    let registry = ProviderRegistry::from_config(config).expect("registry");

    // Then the resolved provider carries the extra_body.
    let resolved = registry
        .get(&ProviderId::new("zai/glm-5.1".to_owned()))
        .expect("resolved");
    let extra = resolved.extra_body.as_ref().expect("extra_body");
    assert_eq!(extra["enable_thinking"], true);
    assert_eq!(extra["tool_stream"], true);
}

#[rstest::rstest]
fn registry_propagates_none_extra_body_when_absent() {
    // Given a config without extra_body.
    let config = make_config(vec![ollama_entry()], vec![], None);

    // When building the registry.
    let registry = ProviderRegistry::from_config(config).expect("registry");

    // Then the resolved provider has None for extra_body.
    let resolved = registry
        .get(&ProviderId::new("ollama/llama3".to_owned()))
        .expect("resolved");
    assert!(resolved.extra_body.is_none());
}

// --- merge_cache tests ---

#[rstest::rstest]
fn merge_cache_adds_remote_entries() {
    // Given a registry with ollama (static: llama3).
    let config = make_config(vec![ollama_entry()], vec![], None);
    let mut registry = ProviderRegistry::from_config(config).expect("registry");

    let mut cache_entries = std::collections::HashMap::new();
    cache_entries.insert(
        "ollama".to_owned(),
        vec![crate::feat::provider_infra::ModelInfo {
            id: "mistral".to_owned(),
            context_length: None,
        }],
    );
    let cache = crate::feat::provider_infra::ModelCache {
        entries: cache_entries,
        last_updated_at: None,
    };

    // When merging the cache.
    registry.merge_cache(&cache);

    // Then the remote model is in the registry.
    assert_eq!(registry.providers().len(), 2);
    let remote = registry
        .get(&ProviderId::new("ollama/mistral".to_owned()))
        .expect("remote entry");
    assert!(remote.is_remote);
    assert_eq!(remote.model, "mistral");
}

#[rstest::rstest]
fn merge_cache_static_wins_on_collision() {
    // Given a registry with ollama (static: llama3).
    let config = make_config(vec![ollama_entry()], vec![], None);
    let mut registry = ProviderRegistry::from_config(config).expect("registry");

    let mut cache_entries = std::collections::HashMap::new();
    cache_entries.insert(
        "ollama".to_owned(),
        vec![crate::feat::provider_infra::ModelInfo {
            id: "llama3".to_owned(),
            context_length: None,
        }],
    );
    let cache = crate::feat::provider_infra::ModelCache {
        entries: cache_entries,
        last_updated_at: None,
    };

    // When merging a cache that has the same model as a static entry.
    registry.merge_cache(&cache);

    // Then the static entry wins (is_remote is still false).
    assert_eq!(registry.providers().len(), 1);
    let entry = registry
        .get(&ProviderId::new("ollama/llama3".to_owned()))
        .expect("entry");
    assert!(!entry.is_remote);
}

#[rstest::rstest]
fn merge_cache_sets_is_remote_true() {
    // Given a registry with ollama.
    let config = make_config(vec![ollama_entry()], vec![], None);
    let mut registry = ProviderRegistry::from_config(config).expect("registry");

    let mut cache_entries = std::collections::HashMap::new();
    cache_entries.insert(
        "ollama".to_owned(),
        vec![crate::feat::provider_infra::ModelInfo {
            id: "deepseek-v3".to_owned(),
            context_length: None,
        }],
    );
    let cache = crate::feat::provider_infra::ModelCache {
        entries: cache_entries,
        last_updated_at: None,
    };

    // When merging.
    registry.merge_cache(&cache);

    // Then static entries have is_remote=false, cached have is_remote=true.
    let static_entry = registry
        .get(&ProviderId::new("ollama/llama3".to_owned()))
        .expect("static");
    assert!(!static_entry.is_remote);

    let remote_entry = registry
        .get(&ProviderId::new("ollama/deepseek-v3".to_owned()))
        .expect("remote");
    assert!(remote_entry.is_remote);
}

#[rstest::rstest]
fn merge_cache_ignores_unknown_provider() {
    // Given a registry with ollama.
    let config = make_config(vec![ollama_entry()], vec![], None);
    let mut registry = ProviderRegistry::from_config(config).expect("registry");

    let mut cache_entries = std::collections::HashMap::new();
    cache_entries.insert(
        "unknown-provider".to_owned(),
        vec![crate::feat::provider_infra::ModelInfo {
            id: "model".to_owned(),
            context_length: None,
        }],
    );
    let cache = crate::feat::provider_infra::ModelCache {
        entries: cache_entries,
        last_updated_at: None,
    };

    // When merging a cache with an unknown provider.
    registry.merge_cache(&cache);

    // Then no new entries are added.
    assert_eq!(registry.providers().len(), 1);
}

#[rstest::rstest]
fn unavailable_providers_returns_correct_entries() {
    // Kills: replace unavailable_providers with vec![] and delete ! in filter.
    // Given a registry with one keyless and one key-required provider (no key).
    let config = make_config(vec![ollama_entry(), openrouter_entry()], vec![], None);
    let registry = ProviderRegistry::from_config(config).expect("registry");
    let api_keys = ApiKeys::new();

    // When asking for unavailable providers.
    let unavailable = registry.unavailable_providers(&api_keys);

    // Then only the key-required provider is in the unavailable list.
    assert_eq!(unavailable.len(), 1);
    assert_eq!(unavailable[0].name, "openrouter");
    assert_eq!(unavailable[0].model, "gpt-4");
}

#[rstest::rstest]
fn unavailable_providers_returns_empty_when_all_available() {
    // Kills: delete ! in filter (would return available instead).
    // Given a registry with only keyless providers.
    let config = make_config(vec![ollama_entry()], vec![], None);
    let registry = ProviderRegistry::from_config(config).expect("registry");
    let api_keys = ApiKeys::new();

    // When asking for unavailable providers.
    let unavailable = registry.unavailable_providers(&api_keys);

    // Then the list is empty (all are available).
    assert!(unavailable.is_empty());
}

#[rstest::rstest]
fn create_factory_returns_injected_override() {
    // Given a registry whose resolved provider would normally map to the
    // 'sample' backend, but an override factory is injected.
    let config = make_config(
        vec![ProviderEntry {
            name: "sample".to_owned(),
            backend: "sample".to_owned(),
            models: vec!["sample".to_owned()],
            base_url: None,
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        vec![],
        None,
    );
    let registry = ProviderRegistry::from_config(config)
        .expect("registry")
        .with_factory_override(
            std::sync::Arc::new(crate::FakeLlmServiceFactory::new(vec![]))
                as std::sync::Arc<dyn super::service::LlmServiceFactory>,
        );
    let api_keys = ApiKeys::new();

    // When creating a factory.
    let factory = registry.create_factory(&ProviderId::new("sample/sample".to_owned()), &api_keys);

    // Then the injected override (not the sample backend) is returned.
    assert!(factory.is_ok());
    assert_eq!(factory.unwrap().name(), "FakeLlm");
}

#[rstest::rstest]
fn create_factory_falls_through_to_production_when_no_override() {
    // Given a registry with a sample provider and NO override.
    let config = make_config(
        vec![ProviderEntry {
            name: "sample".to_owned(),
            backend: "sample".to_owned(),
            models: vec!["sample".to_owned()],
            base_url: None,
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        vec![],
        None,
    );
    let registry = ProviderRegistry::from_config(config).expect("registry");
    let api_keys = ApiKeys::new();

    // When creating a factory.
    let factory = registry.create_factory(&ProviderId::new("sample/sample".to_owned()), &api_keys);

    // Then production behavior is preserved (sample backend, not an override).
    assert!(factory.is_ok());
    assert_eq!(factory.unwrap().name(), "Sample");
}

#[rstest::rstest]
fn from_config_leaves_factory_override_none() {
    // Given a registry built from config.
    let config = make_config(vec![ollama_entry()], vec![], None);
    let registry = ProviderRegistry::from_config(config).expect("registry");

    // Then no factory override is set (production unchanged).
    assert!(registry.factory_override.is_none());
}
