//! OpenRouter endpoint picker entries and compatibility export.
//!
//! The portable endpoint value is owned by `jinn-core-types`. The picker entry
//! remains here because it depends on selection-widget, theme, and ratatui.

pub mod picker_entry;

pub use picker_entry::EndpointEntry;
