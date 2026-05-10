//! Rendering layer for the component system.
//!
//! This crate defines [`UiElement`] — the trait for drawable UI components —
//! and [`UiRegistry`] — the collection that holds them for the render loop.
//! Components register elements during startup, and the TUI layer iterates
//! them each frame.
//!
//! # Elements
//!
//! Elements implement [`UiElement`] and read state during rendering.
//! They are registered during startup via [`UiRegistry`] and iterated
//! by the render loop each frame.
//!
//! # Architecture
//!
//! ```text
//! nullslop-component-core     (bus, message queue)
//!       │
//!       ▼
//! nullslop-component-ui       (UiElement trait + UiRegistry)
//!       │
//!       ▼
//! nullslop-component          (built-in components implement UiElement)
//!       │
//!       ▼
//! nullslop-tui             (discovers UiElements via registry, renders them)
//! ```

pub mod element;
pub mod fake;
pub mod registry;

pub use element::UiElement;
pub use registry::UiRegistry;
