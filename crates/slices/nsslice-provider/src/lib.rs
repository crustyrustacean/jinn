//! Provider slice — streaming indicator and message queue display.
//!
//! Two display-only elements:
//!
//! - **Streaming indicator** — animated throbber shown during sending/streaming.
//! - **Queue display** — dimmed "QUEUED:" entries for messages waiting in the queue.

pub mod indicator;
pub mod queue_element;

pub use indicator::StreamingIndicatorElement;
pub use queue_element::QueueDisplayElement;

use nullslop_component::AppUiRegistry;

/// Register provider UI elements.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(StreamingIndicatorElement::new()));
    registry.register(Box::new(QueueDisplayElement));
}
