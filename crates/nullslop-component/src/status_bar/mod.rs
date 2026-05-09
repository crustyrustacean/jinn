//! Status bar — displays the currently active model and provider.
//!
//! A display-only component at the bottom of the screen showing which
//! provider/model is active for the current session. Shows "no model selected"
//! when no provider has been configured.

pub mod element;

pub use element::StatusBarElement;
