//! Lua workflow integration module.
//!
//! Bridges the [`jinn_lua_workflow`] crate to the domain layer, providing
//! a host handler that processes [`HostRequest`] variants against the
//! domain's session state and LLM infrastructure.

pub mod host_handler;

pub use host_handler::LuaHostHandler;
