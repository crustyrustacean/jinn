//! CWD input popup - type a directory path to change the session cwd.
//!
//! Provides a centered text-input popup (no TUI suspend) as a fast alternative to
//! the fzf/suspend flow bound to `<M-c>`/`<M-d>`. The user types a path; the footer
//! shows a live resolved path (green check) or error (red x). On confirm, the path
//! is resolved (`~` expand, relative-to-cwd, canonicalize), validated as an existing
//! dir, and the active session cwd is updated with context rescanned inline.

pub mod intent;
pub mod render;
pub mod resolve;
pub mod state;
