//! The domain layer — protocol types, actors, intents, and UI elements.
//!
//! This crate consolidates all domain types and logic into a single crate:
//!
//! - **Protocol types** (`protocol/`) — Command/Event mega-enums, foundational
//!   value types (ChatEntry, SessionId, Key, Mode, etc.)
//! - **Component UI** (`component_ui/`) — UiElement trait and registry
//! - **Domain slices** — actors, intents, UI elements, and state for each
//!   domain (provider, session, context, tools, etc.)
//!
//! Protocol types are re-exported at the crate root for convenience.

pub mod char_counter;
pub mod chat_entry_selection;
pub mod chat_input_box;
pub mod chat_log;
pub mod chat_session;
pub mod component_ui;
pub mod context;
pub mod dashboard;
pub mod echo;
pub mod global;
pub mod llm;
pub mod navigation;
pub mod picker;
pub mod pinned_panel;
pub mod protocol;
pub mod provider;
pub mod session;
pub mod shutdown;
pub mod status_bar;
pub mod tools;
