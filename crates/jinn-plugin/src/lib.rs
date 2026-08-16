//! Plugin runner — the process and transport layer for jinn plugins.
//!
//! A plugin runs as a child of jinn's own executable (`jinn
//! --serve-wasm-plugin`) instantiating a WASM guest, and speaks the v1 wire
//! contract (NDJSON over stdin/stdout) defined by [`jinn_plugin_api`].
//! This crate owns:
//!
//! - [`Grants`] — the resolved capability set a plugin is allowed
//!   (filesystem paths, network, plugin-specific config), with template
//!   expansion (`<config_dir>`, `<data_dir>`, `<plugin_data_dir>`).
//! - [`PluginProcess`] — the spawned runner child: piped stdio, bounded
//!   stderr ring, line-capped NDJSON write side.
//! - NDJSON framing helpers shared by the host and the runner child itself.
//!
//! Trust decisions live **upstream** in the plugin coordinator
//! (jinn-domain): this layer moves bytes and frames lines, it does not
//! decide what is allowed.

pub mod engine;
mod framing;
mod grants;
mod process;
mod stderr_ring;

pub use framing::{FramingError, MAX_LINE_BYTES, decode_envelope, encode_envelope};
pub use grants::{
    DirContext, Grants, GrantsError, PathGrant, TemplateVariable, expand_template, resolve_grants,
};
pub use process::{PluginProcess, SpawnInfo};
pub use stderr_ring::StderrRing;
