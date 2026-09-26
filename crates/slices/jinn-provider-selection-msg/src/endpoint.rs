//! OpenRouter endpoint picker entries and compatibility export.
//!
//! The portable endpoint value is owned by `jinn-core-types`. The picker entry
//! remains here because it depends on selection-widget, theme, and ratatui.

pub mod picker_entry;
pub mod picker_scope;
pub mod picker_state;

pub use picker_entry::{AUTO_ROUTE_SENTINEL_TAG, EndpointEntry};
pub use picker_scope::endpoint_picker_scope;
pub use picker_state::{EndpointPickerState, RESULTS_VIEWPORT_FALLBACK, endpoint_picker_slot};
