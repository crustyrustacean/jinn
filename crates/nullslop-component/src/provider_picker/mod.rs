//! Provider picker component — filter and select LLM providers.
//!
//! Manages the picker overlay state for browsing and filtering providers.
//! The picker uses `SelectionState<PickerEntry>` from the
//! `nullslop-selection-widget` crate for all state management and rendering.
//!
//! Phase 5: Handler removed — picker loading will be re-implemented in Phase 7.

pub mod entries;
pub mod loader;

pub use entries::PickerEntry;
pub use loader::load_provider_picker_items;
