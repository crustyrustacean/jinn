//! Feature modules - domain-specific logic, actors, and UI elements.

pub mod chat_entry_selection;
pub mod chat_input;
pub mod context;
pub mod file_lister;
pub mod global;
pub mod image_convert;
pub mod install;
pub mod intent;
pub mod navigation;
pub mod persona;
pub mod picker;
pub mod project;
pub mod provider;
pub use jinn_provider_config as provider_infra;
pub mod session;
pub mod session_lifecycle;
// The search/transcript data model is owned by the session-store family msg
// crate (both the kernel store seam and the store slice consume it). Re-exported
// here so existing kernel paths keep resolving.
pub use jinn_session_store_msg as session_search;
pub mod skills;
pub mod theme;

pub mod ui;
