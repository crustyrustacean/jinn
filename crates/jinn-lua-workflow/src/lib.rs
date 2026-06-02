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

pub mod protocol;
pub mod registry;

pub use protocol::HostRequest;
pub use registry::{LuaError, LuaRegistry, VmHandle};
pub mod vm;
