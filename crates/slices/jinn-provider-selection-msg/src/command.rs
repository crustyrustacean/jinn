//! Provider commands — re-homed verbatim from the kernel
//! `feat/provider/protocol/command.rs` (minus the deleted `SendMessage`
//! shim and the prompt-scan pair, which lives in `jinn-session-init-msg`).

use jinn_core_types::ModelSelection;
use jinn_core_types::SessionId;
use serde::{Deserialize, Serialize};

use jinn_slices::BusMessage;

/// Switch the active LLM provider.
///
/// Carries the target provider ID. The handler validates it against the registry,
/// swaps the factory, and emits [`ProviderSwitched`](crate::ProviderSwitched).
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Switch a session's model selection.")]
pub struct ProviderSwitch {
    /// The session to switch provider for.
    pub session_id: SessionId,
    /// The model selection to switch to.
    pub provider_id: ModelSelection,
}

impl BusMessage for ProviderSwitch {}

/// Refresh the model list from all providers.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Refresh the model list from all providers.")]
pub struct RefreshModels;
impl BusMessage for RefreshModels {}

/// Load entries for the provider/model picker.
///
/// The provider actor receives this, loads entries from the provider registry,
/// and writes them into the provider cell.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Load provider picker entries from the registry.")]
pub struct LoadProviderPickerEntries;

impl BusMessage for LoadProviderPickerEntries {}

/// The provider actor receives this, resolves the active session's model
/// backend, and either fetches the model's OpenRouter routing endpoints via
/// `list_endpoints` or — for a non-OpenRouter backend — populates a single
/// explanatory "not served via OpenRouter" row. The entries are written into
/// the provider cell's endpoint picker.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Load endpoint picker entries for the active model.")]
pub struct LoadEndpointPickerEntries;

impl BusMessage for LoadEndpointPickerEntries {}

/// Force-refresh the OpenRouter endpoint picker entries for the active model,
/// bypassing the in-memory cache (used by the `<c-r>` keybind).
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Force-refresh endpoint picker entries, bypassing the cache.")]
pub struct RefreshEndpointPickerEntries;

impl BusMessage for RefreshEndpointPickerEntries {}
