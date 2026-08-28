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
//!
pub mod emulator;
pub mod pty_session;
pub mod query_responder;
pub mod settle;

pub use emulator::Emulator;
pub use pty_session::PtySession;
