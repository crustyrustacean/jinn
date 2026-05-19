#![allow(clippy::expect_used, clippy::indexing_slicing)]

use crate::feat::provider_infra::config::ProvidersConfig;
use crate::feat::provider_infra::registry_service::ProviderRegistryService;

#[rstest::rstest]
fn clone_sees_same_providers() {
    // Given a service with one provider.
    let config = ProvidersConfig {
        providers: vec![crate::feat::provider_infra::config::ProviderEntry {
            name: "ollama".to_owned(),
            backend: "ollama".to_owned(),
            models: vec!["llama3".to_owned()],
            base_url: None,
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        aliases: vec![],
        default_provider: None,
    };
    let registry = crate::feat::provider_infra::registry::ProviderRegistry::from_config(config)
        .expect("registry");
    let service = ProviderRegistryService::new(registry);
    let clone = service.clone();

    // When reading from both.
    let original_providers = service.providers();
    let cloned_providers = clone.providers();

    // Then both see the same data.
    assert_eq!(original_providers.len(), 1);
    assert_eq!(cloned_providers.len(), 1);
    assert_eq!(original_providers[0].name, "ollama");
    assert_eq!(cloned_providers[0].name, "ollama");
}

/// Helper: build a service with an ollama (keyless) and openrouter (key-required) provider.
fn service_with_providers() -> ProviderRegistryService {
    let config = ProvidersConfig {
        providers: vec![
            crate::feat::provider_infra::config::ProviderEntry {
                name: "ollama".to_owned(),
                backend: "ollama".to_owned(),
                models: vec!["llama3".to_owned()],
                base_url: None,
                api_key_env: None,
                requires_key: false,
                extra_body: None,
                context_length: None,
            },
            crate::feat::provider_infra::config::ProviderEntry {
                name: "openrouter".to_owned(),
                backend: "openrouter".to_owned(),
                models: vec!["gpt-4".to_owned()],
                base_url: None,
                api_key_env: Some("OPENROUTER_API_KEY".to_owned()),
                requires_key: true,
                extra_body: None,
                context_length: None,
            },
        ],
        aliases: vec![crate::feat::provider_infra::config::AliasEntry {
            name: "fast".to_owned(),
            target: "ollama/llama3".to_owned(),
        }],
        default_provider: None,
    };
    let registry = crate::feat::provider_infra::registry::ProviderRegistry::from_config(config)
        .expect("registry");
    ProviderRegistryService::new(registry)
}

#[rstest::rstest]
fn providers_returns_first_model() {
    // Given a service with two providers.
    let service = service_with_providers();

    // When calling providers().
    let providers = service.providers();

    // Then the first expanded provider is returned.
    assert_eq!(providers.len(), 2);
    assert_eq!(providers[0].name, "ollama");
    assert_eq!(providers[0].model, "llama3");
}

#[rstest::rstest]
fn providers_returns_second_model() {
    // Given a service with two providers.
    let service = service_with_providers();

    // When calling providers().
    let providers = service.providers();

    // Then the second expanded provider is returned.
    assert_eq!(providers.len(), 2);
    assert_eq!(providers[1].name, "openrouter");
    assert_eq!(providers[1].model, "gpt-4");
}

#[rstest::rstest]
fn aliases_delegates_to_registry() {
    // Given a service with one alias.
    let service = service_with_providers();

    // When calling aliases().
    let aliases = service.aliases();

    // Then the alias is returned.
    assert_eq!(aliases.len(), 1);
    assert_eq!(aliases[0].name, "fast");
    assert_eq!(aliases[0].target, "ollama/llama3");
}

#[rstest::rstest]
fn get_returns_entry_for_known_provider() {
    // Given a service with providers.
    let service = service_with_providers();

    // When looking up a known provider by full expanded ID.
    let entry = service.get(&crate::feat::provider_infra::provider_id::ProviderId::new(
        "ollama/llama3".to_owned(),
    ));

    // Then the resolved provider is returned with the correct name and model.
    assert!(entry.is_some());
    let entry = entry.unwrap();
    assert_eq!(entry.name, "ollama");
    assert_eq!(entry.model, "llama3");
}

#[rstest::rstest]
fn get_returns_none_for_unknown() {
    // Given a service with providers.
    let service = service_with_providers();

    // When looking up an unknown provider.
    let entry = service.get(&crate::feat::provider_infra::provider_id::ProviderId::new(
        "nonexistent/model".to_owned(),
    ));

    // Then None is returned.
    assert!(entry.is_none());
}

#[rstest::rstest]
fn is_available_delegates_to_registry() {
    // Given a service with a keyless provider.
    let service = service_with_providers();
    let api_keys = crate::feat::provider_infra::api_keys::ApiKeys::new();

    // When checking availability of the keyless provider.
    let id = crate::feat::provider_infra::provider_id::ProviderId::new("ollama/llama3".to_owned());

    // Then it is available.
    assert!(service.is_available(&id, &api_keys));
}

#[rstest::rstest]
fn resolve_alias_delegates_to_registry() {
    // Given a service with an alias.
    let service = service_with_providers();

    // When resolving the alias.
    let resolved = service.resolve_alias("fast");

    // Then the target resolved provider is returned.
    assert!(resolved.is_some());
    let resolved = resolved.unwrap();
    assert_eq!(resolved.name, "ollama");
    assert_eq!(resolved.model, "llama3");
}

#[rstest::rstest]
fn default_provider_id_delegates_to_registry() {
    // Given a service with a configured default provider.
    let config = ProvidersConfig {
        providers: vec![crate::feat::provider_infra::config::ProviderEntry {
            name: "ollama".to_owned(),
            backend: "ollama".to_owned(),
            models: vec!["llama3".to_owned()],
            base_url: None,
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        aliases: vec![],
        default_provider: Some("ollama/llama3".to_owned()),
    };
    let registry = crate::feat::provider_infra::registry::ProviderRegistry::from_config(config)
        .expect("registry");
    let service = ProviderRegistryService::new(registry);

    // When asking for the default provider.
    let id = service.default_provider_id();

    // Then the configured default is returned.
    assert!(id.is_some());
    assert_eq!(
        id.as_ref()
            .map(crate::feat::provider_infra::provider_id::ProviderId::as_str),
        Some("ollama/llama3")
    );
}

#[rstest::rstest]
fn create_factory_delegates_to_registry() {
    // Given a service with a sample provider.
    let config = ProvidersConfig {
        providers: vec![crate::feat::provider_infra::config::ProviderEntry {
            name: "sample".to_owned(),
            backend: "sample".to_owned(),
            models: vec!["sample".to_owned()],
            base_url: None,
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        aliases: vec![],
        default_provider: None,
    };
    let registry = crate::feat::provider_infra::registry::ProviderRegistry::from_config(config)
        .expect("registry");
    let service = ProviderRegistryService::new(registry);
    let api_keys = crate::feat::provider_infra::api_keys::ApiKeys::new();

    // When creating a factory via the service.
    let id = crate::feat::provider_infra::provider_id::ProviderId::new("sample/sample".to_owned());
    let factory = service.create_factory(&id, &api_keys);

    // Then it succeeds and returns a factory named "Sample".
    assert!(factory.is_ok());
    assert_eq!(factory.unwrap().name(), "Sample");
}

#[rstest::rstest]
fn set_default_provider_updates_via_service() {
    // Given a service with a provider.
    let service = service_with_providers();

    // When setting the default provider.
    service.set_default_provider(Some("ollama/llama3".to_owned()));

    // Then default_provider_id returns the updated value.
    let id = service.default_provider_id();
    assert!(id.is_some());
    assert_eq!(
        id.as_ref()
            .map(crate::feat::provider_infra::provider_id::ProviderId::as_str),
        Some("ollama/llama3")
    );
}

#[rstest::rstest]
fn config_snapshot_returns_current_config() {
    // Given a service with providers.
    let service = service_with_providers();

    // When modifying and taking a snapshot.
    service.set_default_provider(Some("ollama/llama3".to_owned()));
    let config = service.config_snapshot();

    // Then the snapshot reflects the change.
    assert_eq!(config.providers.len(), 2);
    assert_eq!(config.default_provider.as_deref(), Some("ollama/llama3"));
}
