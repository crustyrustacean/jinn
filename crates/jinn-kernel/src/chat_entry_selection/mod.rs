//! The chat entry selection cursor.
//!
//! This is the machinery behind "which entry is selected, and what happens when
//! the user moves it": the selection index, its scroll position, pinning,
//! forking, and the ignore sweep. It mutates `AppState` directly and publishes
//! messages to several different slices, which is the shape of kernel dispatch
//! rather than slice logic. It lives outside `feat/` for the same reason
//! `session_lifecycle` does: every caller is kernel code, and a slice that
//! owned it would have to depend on the kernel to reach it.
//!
//! Rendering stays in `jinn-tui`; this module has no element.

pub mod ignore_sweep;
pub mod intent;
pub mod isolate;
pub mod validator;
