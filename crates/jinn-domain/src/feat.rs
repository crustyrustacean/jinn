//! Feature modules - domain-specific logic, actors, and UI elements.

pub mod auto_prune_worker;
pub mod chat_entry_selection;
pub mod chat_input;
pub mod compaction_worker;
pub mod context;
pub mod global;
pub mod history_worker;
pub mod intent;

pub mod llm_actor;
pub mod navigation;
pub mod persona;
pub mod picker;
pub mod preferences_actor;
pub mod provider;
pub mod provider_infra;
pub mod queue_actor;
pub mod rename_session_input;
pub mod session;
pub mod session_lifecycle;
pub mod sidebar_resize;
pub mod skills;
pub mod todo_list;
pub mod theme;
pub mod token_count_actor;
pub mod tools_actor;
pub mod web_fetch_actor;
pub mod ui;
pub mod workflow;
