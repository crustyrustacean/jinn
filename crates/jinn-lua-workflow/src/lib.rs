//! Lua workflow system — async-first scripting for jinn workflows.
//!
//! Each workflow script runs in its own `mlua` VM on a tokio task.
//! Communication flows through channels — no `!Send` constraints on the registry.
//! The host provides a **ctx table** per invocation carrying exactly the data
//! and capabilities the script needs.
//!
//! Key types:
//! - [`LuaRegistry`] — manages VM handles (no `Lua` instances, fully `Send + Sync`)
//! - [`VmHandle`] — channel sender + join handle for a running VM task
//! - [`CtxBuilder`] — builds the ctx table with data and capability methods
//!
//! Capability functions:
//! - [`make_llm`] — creates `ctx.llm(prompt)` async function
//! - [`make_push_user`] — creates `ctx.push_user(text)` async function
//! - [`make_push_system`] — creates `ctx.push_system(text)` async function
//! - [`make_turn_off`] — creates `ctx.turn_off()` async function
//! - [`make_gather`] — creates `ctx.gather(fns)` async function

pub mod capabilities;
pub mod ctx;
pub mod protocol;
pub mod registry;
pub mod vm;

pub use capabilities::{make_gather, make_llm, make_push_system, make_push_user, make_turn_off};
pub use ctx::CtxBuilder;
pub use protocol::HostRequest;
pub use registry::{LuaError, LuaRegistry, VmHandle};
pub use vm::{CtxConfig, spawn_one_shot};
