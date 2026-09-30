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

/// The user confirmed a choice in the OpenRouter endpoint picker.
///
/// The pin is a per-model default in `providers.toml`, not session state: the
/// provider actor persists a `[[endpoint_defaults]]` row for `model` and then
/// writes it back into the registry it holds. A `None` `tag` is the
/// auto-route sentinel and removes the row.
///
/// The route action cannot do this itself — an `ActionCtx` carries only app
/// state, slices, and the config layer, never `Services` — so confirming
/// publishes this command and the actor performs the write.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Persist the chosen OpenRouter routing endpoint as a per-model default.")]
pub struct SetEndpointDefault {
    /// Full model id the choice applies to (`{provider}/{model}`).
    pub model: String,
    /// The routing tag to pin, or `None` to return the model to auto-route.
    pub tag: Option<String>,
}

impl BusMessage for SetEndpointDefault {}
