//! Chat log slice — renders the full conversation history.
//!
//! A display-only component showing all messages exchanged in the active session.
//! Each entry type has a distinct visual style (user bold with `>`, system dark gray,
//! actor yellow, assistant cyan). Supports scrolling, selection highlighting,
//! and pinned entry indicators.
//!
//! # Slice Convention
//!
//! Display-only slices (like this one) contain only an element module.
//! Intent-bearing slices extend this pattern:
//!
//! ```text
//! nsslice-<feature>/
//! ├── src/
//! │   ├── lib.rs        — register() + re-exports
//! │   ├── element.rs    — UiElement impl + tests
//! │   ├── intent.rs     — pub fn handle_<intent>(state: &mut AppState) -> IntentResult
//! │   └── validator.rs  — pub fn validate_<intent>(state: &AppState) -> Result<(), Error> + tests
//! ```
//!
//! The central `IntentHandler::handle()` match block in `nullslop-intent`
//! calls into slice handler functions. `IntentResult` lives in `nullslop-protocol`.

pub mod element;

pub use element::ChatLogElement;

use nullslop_component::AppUiRegistry;

/// Register chat log UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(ChatLogElement));
}
