//! OpenRouter endpoint loading — target resolution, fetch, and entry
//! building.
//!
//! Re-homed from the kernel `feat/provider/loader.rs` (the endpoint
//! half). The fetch has no fake seam — `list_endpoints_default_client`
//! is a direct HTTP call — so the entry-building functions are pure and
//! the fetch stays behind the actor's cache.

use jinn_core_types::ModelSelection;
use jinn_provider_selection_msg::endpoint::Endpoint;
use jinn_provider_selection_msg::endpoint::EndpointEntry;

use jinn_domain::Services;
use jinn_domain::feat::provider_infra as infra;

/// The credentials + model id needed to query OpenRouter's `/endpoints`.
///
/// Returned by [`resolve_openrouter_target`] only when the active session's
/// model is `Single` and its configured backend resolves to OpenRouter.
pub(crate) struct OpenRouterTarget {
    model_id: String,
    base_url: String,
    api_key: String,
}

impl OpenRouterTarget {
    /// The resolved model id — used as the per-model endpoint cache key.
    pub(crate) fn model_id(&self) -> &str {
        &self.model_id
    }
}

/// Resolves whether the active session's model is served via OpenRouter.
///
/// Returns `Some(target)` only when the model selection is `Single` and the
/// resolved provider's backend parses to `Backend::OpenRouter`. Otherwise
/// returns `None` — the picker then renders a single "not served via
/// OpenRouter" explanatory row.
///
/// This is the backend gate: it runs in the actor (which owns `Services`)
/// rather than the validator, because the `IntentHandler` cannot reach the
/// provider registry.
pub(crate) fn resolve_openrouter_target(
    services: &Services,
    model: &ModelSelection,
) -> Option<OpenRouterTarget> {
    let ModelSelection::Single(provider_id) = model else {
        return None;
    };
    let registry = services.provider_registry.read();
    let resolved = registry.get(&infra::ProviderId::new(provider_id.clone()))?;
    if resolved.backend.parse::<jinn_provider::Backend>().ok()?
        != jinn_provider::Backend::OpenRouter
    {
        return None;
    }
    let api_key = if resolved.requires_key {
        let env_var = resolved.api_key_env.as_ref()?;
        services.api_keys.get(env_var)?
    } else {
        String::new()
    };
    let base_url = resolved.base_url.clone().unwrap_or_else(|| {
        jinn_provider::ProviderConfig::openrouter()
            .default_base_url
            .to_owned()
    });
    Some(OpenRouterTarget {
        model_id: resolved.model.clone(),
        base_url,
        api_key,
    })
}

/// Fetches the OpenRouter endpoints for `target` from the network.
///
/// Returns `Ok(endpoints)` on success or `Err(())` on any failure (network,
/// HTTP, parse). The caller decides what to render from the outcome.
pub(crate) async fn fetch_endpoints(
    target: &OpenRouterTarget,
) -> Result<Vec<jinn_provider::EndpointInfo>, ()> {
    jinn_provider::list_endpoints_default_client(
        &target.base_url,
        &target.model_id,
        &target.api_key,
        &[],
    )
    .await
    .map_err(|_e| ())
}

/// Builds the picker entries from a list of upstream endpoints.
///
/// Always prepends the "Default (auto-route)" sentinel; real endpoints are
/// mapped one-to-one. The currently pinned endpoint, if any, is marked
/// `is_active`. This is pure: same inputs always produce the same entries,
/// so both the fresh-fetch path and the cache-hit path reuse it.
pub(crate) fn build_endpoint_entries(
    endpoints: &[jinn_provider::EndpointInfo],
    theme: &jinn_domain::feat::theme::Theme,
    pinned: Option<&Endpoint>,
) -> Vec<EndpointEntry> {
    let mut entries = Vec::with_capacity(endpoints.len() + 1);
    entries.push(EndpointEntry::auto_route(pinned.is_none(), theme.clone()));
    for ep in endpoints {
        let is_active = pinned.is_some_and(|p| p.tag == ep.tag);
        entries.push(EndpointEntry {
            tag: ep.tag.clone(),
            provider_name: ep.provider_name.clone(),
            uptime_30m: ep.uptime_30m,
            prompt_price: ep.prompt_price.clone(),
            completion_price: ep.completion_price.clone(),
            quantization: ep.quantization.clone(),
            max_completion_tokens: ep.max_completion_tokens,
            is_active,
            theme: theme.clone(),
        });
    }
    entries
}

/// Builds the single-row list shown when the active model is not served via
/// OpenRouter (or is an alloy). The row is an inert explanatory placeholder;
/// its empty `tag` means confirming it clears any pin.
pub(crate) fn unavailable_endpoint_entries(
    theme: jinn_domain::feat::theme::Theme,
    pinned: Option<&Endpoint>,
) -> Vec<EndpointEntry> {
    vec![EndpointEntry::auto_route(pinned.is_none(), theme)]
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
    use super::*;
    use jinn_domain::feat::theme::default_theme;

    #[rstest::rstest]
    fn build_endpoint_entries_prepends_sentinel_and_marks_pinned_active() {
        // Given two upstream endpoints and a pin on the second one's tag.
        let theme = default_theme();
        let endpoints = vec![
            jinn_provider::EndpointInfo {
                tag: "azure".to_owned(),
                provider_name: "Azure".to_owned(),
                uptime_30m: Some(99.9),
                prompt_price: None,
                completion_price: None,
                quantization: None,
                max_completion_tokens: None,
            },
            jinn_provider::EndpointInfo {
                tag: "anthropic".to_owned(),
                provider_name: "Anthropic".to_owned(),
                uptime_30m: Some(99.7),
                prompt_price: None,
                completion_price: None,
                quantization: None,
                max_completion_tokens: None,
            },
        ];
        let pinned = Endpoint {
            tag: "anthropic".to_owned(),
            provider_name: "Anthropic".to_owned(),
        };

        // When building entries (pure: same inputs always produce same output).
        let entries = build_endpoint_entries(&endpoints, &theme, Some(&pinned));

        // Then the auto-route sentinel is prepended and is NOT active (a pin exists).
        assert!(entries[0].tag.is_empty(), "first entry is the sentinel");
        assert!(!entries[0].is_active, "sentinel inactive when a pin exists");
        // And the pinned endpoint (anthropic) is the only active one.
        let anthropic = entries
            .iter()
            .find(|e| e.tag == "anthropic")
            .expect("anthropic entry");
        assert!(anthropic.is_active, "pinned entry must be active");
        assert_eq!(
            entries.iter().filter(|e| e.is_active).count(),
            1,
            "exactly one active entry"
        );
    }
}
