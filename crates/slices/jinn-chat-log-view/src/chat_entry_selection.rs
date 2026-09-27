//! The chat log's entry actions — the verbs behind every key the log owns.
//!
//! Selection, cursor jumps, pinning, forking, yanking, and the
//! context-override toggles all live here rather than in the kernel: a
//! key in the composed keymap resolves to a route row, and the row's
//! action lands in this module. The kernel holds no chat-log intent
//! variant and names nothing below.

pub mod ignore_sweep;
pub mod intent;
pub mod isolate;
pub mod scroll;
pub mod validator;
