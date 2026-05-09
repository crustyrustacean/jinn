//! Provider component — UI elements for streaming LLM responses and message queuing.
//!
//! `StreamingIndicatorElement` shows the current streaming/sending/queue state.
//! `QueueDisplayElement` shows stacked dimmed "QUEUED:" entries.
//!
//! Phase 5: All handlers removed — streaming, message queuing, prompt assembly,
//! refresh, and switch logic will be re-implemented in Phase 7.

pub mod indicator;
pub mod queue_element;
