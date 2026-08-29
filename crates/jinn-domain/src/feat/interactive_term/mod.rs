//! Interactive terminal sessions — drive TUI programs from the agent.
//!
//! This feature lets the model spawn an interactive program (vim, psql, ssh,
//! REPLs) in a real PTY, send it keystrokes, and read back its rendered
//! screen. Unlike every other tool child (which runs deliberately tty-less via
//! [`crate::feat::tools_actor::bash`] isolation), these children get a PTY as
//! their controlling terminal so full-screen programs work.
//!
//! Layout:
//! - [`pty_session`] — PTY spawn, kill guard, output pump, resize
//! - [`emulator`] — terminal emulation: bytes in, rendered screen out
//! - [`query_responder`] — replies to terminal capability queries so probing
//!   TUIs don't hang at startup
//! - [`settle`] — output settle detection (quiet window, hard cap) and
//!   named-key → byte encoding
//! - [`interactive_term_actor`] — the coordinator owning sessions across
//!   tool calls
//! - [`takeover_intent`] — the IntentHandler arms for the terminal tab
//!   (take control, send key, handback + steering)
//! - [`terminal_tab_state`] — the frontend mirror the actor writes and the
//!   renderer reads
//! - [`prefs`] — `[interactive_term]` config (handback key, settle wait)
//! - [`protocol`] — its ask messages and events
//!
pub mod emulator;
pub mod interactive_term_actor;
pub mod prefs;
pub mod protocol;
pub mod pty_session;
pub mod query_responder;
pub mod settle;
pub mod takeover_intent;
pub mod terminal_tab_state;

pub use emulator::Emulator;
pub use pty_session::PtySession;
