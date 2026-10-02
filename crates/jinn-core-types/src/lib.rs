//! Foundational, domain-agnostic value types shared across the jinn workspace.
//!
//! Residents here are pure value types (newtypes over primitives) with no
//! dependency on domain logic, actors, or app state. They exist so that leaf
//! crates can reference a shared type without depending on `jinn-kernel`.
//!
//! Types are added as-needed. This is not a dumping ground: only types that are
//! both foundational and domain-agnostic belong here.
//!
//! `chat_entry`/`chat_history`/`history_mutation`/`tool_result_status`/
//! `entry_timing` (the `ChatEntry` vocabulary, promoted from the kernel
//! session feature, 2026-09-19) hold only vocabulary: every field is a
//! `jinn-core-types` value or a plain serde scalar. They carry methods —
//! constructors, predicates, content hashing, and the context-override state
//! machine — but every one of those is a question about the value itself, with
//! no dependency on actors, sessions, or app state. What they deliberately do
//! not contain is domain behavior: the `HistoryEditor` write path stays
//! kernel-side, because that mutates session state rather than these types.
//!
//! Three modules split this out by job rather than by kind:
//! `chat_entry_constructors` builds entries, `chat_entry_serde` fixes their
//! on-disk representation, and `chat_entry` itself answers questions about
//! entries that already exist.

pub mod actor_lifecycle;
pub mod attachment;
pub mod chat_entry;
mod chat_entry_constructors;
pub mod chat_entry_id;
mod chat_entry_serde;
pub mod chat_history;
pub mod context_override;
pub mod endpoint;
pub mod entry_timing;
pub mod filter;
pub mod history_mutation;
pub mod llm_message;
pub mod model_selection;
pub mod reasoning;
pub mod session_id;
pub mod session_profile;
pub mod tool_result_status;
pub mod tool_types;
pub mod url_citation;
pub mod working_interval;

#[cfg(test)]
mod chat_entry_tests;

pub use actor_lifecycle::ActorLifecycle;
pub use attachment::Attachment;
pub use chat_entry::{
    AttachmentOutcome, ChangeSource, ChatEntry, ChatEntryKind, ContextChangeEvent, PinPosition,
    ResolvedToken,
};
pub use chat_entry_id::ChatEntryId;
pub use chat_history::ChatHistory;
pub use context_override::ContextOverride;
pub use endpoint::Endpoint;
pub use entry_timing::EntryTiming;
pub use filter::{FilterMode, NameFilter};
pub use history_mutation::HistoryMutation;
pub use llm_message::LlmMessage;
pub use model_selection::{AlloyData, AlloyStrategy, ModelSelection, NO_PROVIDER_ID};
pub use reasoning::ReasoningEffort;
pub use session_id::SessionId;
pub use session_profile::{DEFAULT_PERSONA_NAME, SessionProfile};
pub use tool_result_status::ToolResultStatus;
pub use tool_types::{
    ServerToolType, ToolCall, ToolDefinition, ToolResult, ToolResultPinPosition, TruncatedBy,
    TruncationMeta,
};
pub use url_citation::UrlCitation;
pub use working_interval::{
    CoalescingGap, WorkingInterval, coalesce, format_working_duration, total, union,
};
