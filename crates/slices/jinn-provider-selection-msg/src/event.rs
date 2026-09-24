//! Provider events — re-homed verbatim from the kernel
//! `feat/provider/protocol/event.rs` (minus the prompt-scan pair, which
//! lives in `jinn-session-init-msg`).

use serde::{Deserialize, Serialize};

use jinn_core_types::SessionId;
use jinn_provider::ModelInfo;
use jinn_provider_config::ModelCache;

use jinn_slices::BusMessage;

/// The active provider was switched.
///
/// Emitted after a successful [`ProviderSwitch`](crate::ProviderSwitch) command.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A session switched to a new provider.")]
pub struct ProviderSwitched {
    /// The session that switched provider.
    pub session_id: SessionId,
    /// The display name of the new provider.
    pub provider_name: String,
}

impl BusMessage for ProviderSwitched {}

/// Models refresh completed with results and errors.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "Model discovery finished across providers.")]
pub struct ModelsRefreshed {
    /// The session that triggered the refresh (for routing the result back).
    pub session_id: SessionId,
    /// Provider name to list of discovered model metadata.
    pub results: std::collections::HashMap<String, Vec<ModelInfo>>,
    /// Provider name to error message for providers that failed.
    pub errors: std::collections::HashMap<String, String>,
}

impl BusMessage for ModelsRefreshed {}

/// Model cache loaded from disk at startup.
///
/// Emitted by boot's `ProviderInitActor` after loading the cache from disk.
/// `ProviderActor` handles this by writing the cache into the provider cell
/// and reloading picker entries.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "Model cache was loaded from disk.")]
pub struct ModelCacheLoaded {
    /// The loaded model cache.
    pub cache: ModelCache,
}

impl BusMessage for ModelCacheLoaded {}
