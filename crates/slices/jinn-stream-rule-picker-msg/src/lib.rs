//! Stream-rule picker crossing contracts — the picker's state cell payload,
//! its slot key and scope id, and the entry type one picker row shows.
//!
//! The picker itself lives in the `jinn-stream-rule-picker` slice; this
//! crate is the pure contract: data and pure functions only. It exists so
//! the shared cell catalog can name the picker's payload, and so other
//! slices can ask "is the stream-rule picker open?" without depending on
//! the implementation crate.

mod picker_scope;
mod picker_state;

pub use picker_scope::RESULTS_VIEWPORT_FALLBACK;
pub use picker_scope::stream_rule_picker_scope;
pub use picker_scope::stream_rule_picker_slot;
pub use picker_state::StreamRuleEntry;
pub use picker_state::StreamRulePickerState;
pub use picker_state::build_stream_rule_entries;
pub use picker_state::stream_rule_row;
pub use picker_state::stream_rule_search_text;
