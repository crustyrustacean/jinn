//! Provider-selection crossing contracts — the commands and events other
//! actors and slices consume, plus the provider cell vocabulary.
//!
//! Re-homed from the kernel `feat/provider/protocol/` in the
//! provider-selection window. The crossing-schema ids ("ProviderSwitch",
//! "ModelsRefreshed", …) are unchanged — they derive from the type names,
//! which did not move semantically. The dead `SendMessage` backward-compat
//! shim was deleted outright (zero publishers; the session actor
//! republishes `EnqueueUserMessage`), and the prompt-scan pair
//! (`RescanPromptTemplates`/`PromptTemplatesLoaded`) re-homed to
//! `jinn-session-init-msg`, its producer's crate.
//!
//! The cell vocabulary ([`ProviderCell`], [`provider_state_slot`]) lives
//! here, not in the slice crate, because the *readers* include
//! kernel-resident code (the provider/endpoint picker specs, the TUI
//! renderer) and sibling slices (status-bar, context-curation, boot);
//! the *writer* is the slice's `ProviderActor`. Neither depends on the
//! other.

pub mod cell;
pub mod endpoint;
pub mod entries;
pub mod reasoning;

pub use cell::ProviderCell;
pub use cell::provider_state_slot;
pub use endpoint::EndpointEntry;
pub use entries::ProviderPickerEntry;
pub use entries::pre_check_active_models;
pub use jinn_core_types::{Endpoint, ReasoningEffort};
pub use reasoning::ReasoningEffortEntry;
pub use reasoning::reasoning_row;
pub use reasoning::{
    ReasoningPickerState, reasoning_picker_scope, reasoning_picker_slot, resolve_effort,
};

mod command;
mod event;

pub use command::*;
pub use event::*;
