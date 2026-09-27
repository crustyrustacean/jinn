//! Protocol types for the jinn actor system.
//!
//! This module defines cross-cutting types that are used across feature boundaries:
//!
//!
//! - **[`intent`]** - `Intent` (user-initiated action) and `IntentResult`
//! - **[`system`]** - `KeyDown`, `KeyUp`, `ModeChanged`
//! - Shared `Key`, `KeyEvent`, `Modifiers`, and `Mode` vocabulary from `jinn-slices`
//!
//! Kernel protocol vocabulary and shared core/slice values live here; commands
//! and events owned by a slice are defined directly in that slice's canonical
//! `*-msg` crate.

pub mod intent;
pub mod system;

// Re-export primary types
pub use crate::common::bus::BusMessage;
pub use intent::CwdRoot;
pub use intent::IntentResult;
pub use intent::KernelIntent;
pub use intent::ScopeSignal;
pub use jinn_slices::{Key, KeyEvent, Mode, Modifiers};

// Re-export domain types that are widely used as cross-cutting protocol concerns
pub use jinn_session_init_msg::PromptTemplate;

pub use jinn_provider::LlmMessage;
pub use jinn_slices::picker_kind::PickerKind;

// Re-export domain types used by the picker and UI
pub use jinn_llm_support::entries_to_messages::entries_to_messages;
pub use jinn_provider_selection_msg::ProviderPickerEntry;
// The `ChatEntry` vocabulary is promoted to `jinn-core-types` (serde-only
// value types); these re-exports keep the long-standing `jinn_domain::…`
// paths resolving.
pub use jinn_core_types::chat_history::ChatHistory;
pub use jinn_core_types::{
    AttachmentOutcome, ChangeSource, ChatEntry, ChatEntryId, ChatEntryKind, ContextChangeEvent,
    ContextOverride, EntryTiming, HistoryMutation, PinPosition, ResolvedToken, ToolResultStatus,
};
