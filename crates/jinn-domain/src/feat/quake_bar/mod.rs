//! Quake bar — the global overlay console.
//!
//! A drop-down overlay (a la the Quake/Doom console) pinned to the top of the
//! screen. While the [`FocusScope::QuakeBar`](crate::common::focus::FocusScope)
//! scope is on the stack, it captures every keystroke; only `<esc>` dismisses it.
//!
//! Two writers, two fields on [`QuakeBarState`]:
//! - `input` — the 1-line command input, edited synchronously by the
//!   `IntentHandler` (like every other input popup).
//! - `log`   — the persistent command log, owned solely by the
//!   [`QuakeBarActor`], which is the only writer. Submit routes the typed line
//!   through a [`SubmitQuakeBarCommand`](command::SubmitQuakeBarCommand) so the
//!   actor is the single mutator of the log (future debug commands and event
//!   subscriptions also funnel through the actor).

pub mod command;
pub mod intent;
pub mod quake_bar_actor;
pub mod render;
pub mod state;
