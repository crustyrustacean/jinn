//! Provider commands.

use std::path::PathBuf;

use crate::common::bus::BusMessage;
use serde::{Deserialize, Serialize};

use crate::protocol::SessionId;
use jinn_core_types::model_selection::ModelSelection;

/// Switch the active LLM provider.
///
/// Carries the target provider ID. The handler validates it against the registry,
/// swaps the factory, and emits [`ProviderSwitched`](super::ProviderSwitched).
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Switch a session's model selection.")]
pub struct ProviderSwitch {
    /// The session to switch provider for.
    pub session_id: SessionId,
    /// The model selection to switch to.
    pub provider_id: ModelSelection,
}

impl BusMessage for ProviderSwitch {}

/// Send a message to the AI provider.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Backward-compat send: republished as EnqueueUserMessage.")]
pub struct SendMessage {
    /// The session this message belongs to.
    pub session_id: SessionId,
    /// The message text.
    pub text: String,
}

impl BusMessage for SendMessage {}

/// Refresh the model list from all providers.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Refresh the model list from all providers.")]
pub struct RefreshModels;
impl BusMessage for RefreshModels {}

/// Rescan prompt templates for a specific session.
///
/// Carries the session's cwd: the worker scans user/system plus project-local
/// `.agents/prompts` dirs (most-local wins), and emits `PromptTemplatesLoaded`.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Rescan prompt templates for a session.")]
pub struct RescanPromptTemplates {
    /// The session whose scan this is.
    pub session_id: crate::SessionId,
    /// The working directory driving the scan.
    #[serde(default)]
    pub cwd: PathBuf,
}
impl BusMessage for RescanPromptTemplates {}

/// Load entries for the provider/model picker.
///
/// The provider actor receives this, loads entries from the provider registry,
/// and writes them into `AppState`.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Load provider picker entries from the registry.")]
pub struct LoadProviderPickerEntries;

impl BusMessage for LoadProviderPickerEntries {}

/// The provider actor receives this, resolves the active session's model
/// backend, and either fetches the model's OpenRouter routing endpoints via
/// `list_endpoints` or — for a non-OpenRouter backend — populates a single
/// explanatory "not served via OpenRouter" row. The entries are written into
/// `AppState`'s endpoint picker.
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
