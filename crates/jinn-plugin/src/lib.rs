//! Plugin host — the in-process transport layer for jinn plugins.
//!
//! A plugin is a WASM guest hosted **inside** jinn (no child processes):
//! one shared wasmtime engine, one store per plugin, guest stdio wired to
//! in-memory pipes carrying the v1 wire contract (NDJSON) defined by
//! [`jinn_plugin_api`]. This crate owns:
//!
//! - [`Grants`] — the resolved capability set a plugin is allowed
//!   (filesystem paths, network, plugin-specific config), with template
//!   expansion (`<config_dir>`, `<data_dir>`, `<plugin_data_dir>`).
//! - [`PluginHost`] — a live guest: duplex stdio pipes, bounded stderr
//!   ring, line-capped NDJSON write side, abort-on-drop task.
//! - NDJSON framing helpers shared by the host and the guest SDK.
//!
//! Trust decisions live **upstream** in the plugin coordinator
//! (jinn-domain): this layer moves bytes and frames lines, it does not
//! decide what is allowed.

pub mod engine;
mod framing;
pub mod grants;
mod host;
mod stderr_ring;

pub use engine::{EngineError, PluginEngine};
pub use framing::{FramingError, MAX_LINE_BYTES, decode_envelope, encode_envelope};
pub use grants::{
    DirContext, Grants, GrantsError, PathGrant, TemplateVariable, expand_template, resolve_grants,
};
pub use host::{FakeGuestScript, PluginHost, PluginHostError, PluginReader, SpawnInfo};
pub use stderr_ring::StderrRing;
