//! Protocol types for the jinn actor system.
//!
//! This module defines cross-cutting types that are used across feature boundaries:
//!
//!
//! - **[`intent`]** - `Intent` (user-initiated action) and `IntentResult`
//! - **[`key`]** - `Key`, `KeyEvent`, `Modifiers` (keyboard input types)
//! - **[`mode`]** - `Mode` (application interaction mode)
//! - **[`system`]** - `KeyDown`, `KeyUp`, `ModeChanged`
//!
//! Domain-specific types (session, provider, context, tools, chat input, etc.) live
//! in their feature modules under `feat/` and are re-exported here for convenience.

pub mod intent;
pub mod key;
pub mod mode;
pub mod system;

// Re-export primary types
pub use crate::common::bus::BusMessage;
pub use intent::CwdRoot;
pub use intent::IntentResult;
pub use intent::KernelIntent;
pub use intent::ScopeSignal;
pub use key::{Key, KeyEvent, Modifiers};
pub use mode::Mode;

// Re-export domain types that are widely used as cross-cutting protocol concerns
pub use crate::common::actor::actor_name::ActorName;
pub use jinn_session_init_msg::PromptTemplate;

pub use crate::feat::provider::llm_message::LlmMessage;
pub use crate::feat::session::protocol::session_id::SessionId;
pub use jinn_slices::picker_kind::PickerKind;

// Re-export domain types used by the picker and UI
pub use crate::feat::provider::entries_to_messages::entries_to_messages;
pub use crate::feat::session::picker_entry::SessionTreeEntry;
pub use jinn_provider_selection_msg::ProviderPickerEntry;
// The `ChatEntry` vocabulary is promoted to `jinn-core-types` (serde-only
// value types); these re-exports keep the long-standing `jinn_domain::…`
// paths resolving.
pub use jinn_core_types::chat_history::ChatHistory;
pub use jinn_core_types::{
    AttachmentOutcome, ChangeSource, ChatEntry, ChatEntryId, ChatEntryKind, ContextChangeEvent,
    ContextOverride, EntryTiming, HistoryMutation, PinPosition, ResolvedToken, ToolResultStatus,
};
