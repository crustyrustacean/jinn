//! The provider-selection slice's shared cell vocabulary.
//!
//! [`ProviderCell`] lives in the msg crate (not the slice) because the
//! *readers* include kernel-resident code — the provider/endpoint picker
//! specs (specs live in the kernel by design), the status bar, the
//! multimodal attachment gate, and boot's cache loader. The *writers*
//! are the slice's `ProviderActor`, boot's `ProviderInitActor`, and the
//! IntentHandler-side picker specs (the exempt sync writers for
//! alloy mode and the in-flight flag).
//!
//! Only the slice-owned *data* lives here. The provider/endpoint picker
//! `SelectionState`s stay on the kernel's `PickerStates`: they are the
//! render/navigation surface the picker framework's host lens lends as
//! `&dyn Any` from `&AppState` (the theme precedent — the cell is the
//! source, the kernel picker field is the display surface filled at
//! load/open time).

use jinn_provider_config::ModelCache;
use jinn_slices::SlotKey;

/// The provider slice's cell payload.
///
/// Everything the provider-selection family owns as *data*: the
/// discovered model cache, the provider picker's alloy-selection mode,
/// and the OpenRouter endpoint fetch state (in-flight flag + last
/// fetched timestamp for the status line).
#[derive(Debug, Default)]
pub struct ProviderCell {
    /// Last known model cache from discovery.
    /// OWNER: ProviderActor (ModelsRefreshed / ModelCacheLoaded),
    ///        boot's ProviderInitActor (disk load).
    pub model_cache: Option<ModelCache>,

    /// Whether the provider picker is in alloy-selection mode.
    ///
    /// When `false`, ENTER selects the highlighted model as a single model. When
    /// `true`, TAB toggles models into the alloy set and ENTER force-includes the
    /// highlight then commits (1 model -> Single, 2+ -> Alloy).
    /// OWNER: provider spec (set on open, flipped by `<c-a>`, read on
    /// confirm and by the actor when loading entries) — the exempt
    /// IntentHandler-side writer.
    pub alloy_mode: bool,

    /// True while an endpoint fetch is in flight (open or `<c-r>` refresh).
    /// Set synchronously by the open/refresh intent; cleared by `ProviderActor`
    /// when it writes items back (success or error).
    /// OWNER: endpoint spec (sets) / ProviderActor (clears).
    pub endpoint_loading: bool,

    /// When the endpoint cache for the active model was last populated.
    /// Set by `ProviderActor` on a successful fetch (and preserved on a
    /// cache-served open). Survives across picker opens so the status line
    /// can show "fetched Xs ago".
    /// OWNER: ProviderActor.
    pub endpoint_fetched_at: Option<jiff::Timestamp>,
}

impl ProviderCell {
    /// Whether the provider picker is in alloy-selection mode.
    #[must_use]
    pub fn is_alloy_mode(&self) -> bool {
        self.alloy_mode
    }

    /// Set whether the provider picker is in alloy-selection mode.
    pub fn set_alloy_mode(&mut self, on: bool) {
        self.alloy_mode = on;
    }
}

/// The slot key the provider slice's cell lives under.
#[must_use]
pub fn provider_state_slot() -> SlotKey {
    SlotKey::builtin("provider", "state")
}
