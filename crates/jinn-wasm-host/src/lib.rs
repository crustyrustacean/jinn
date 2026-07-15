//! WASM component host runtime for jinn plugins.
//!
//! Replaces the Lua (`mlua`) backend. Plugins are authored in Rust (the sole
//! supported PDK language) and compiled to WASM components; this crate loads
//! them, wires host-owned state bags, and fires hooks via runtime export
//! lookup.
//!
//! # Architecture
//!
//! - **Engine** (`engine.rs`) — one shared `wasmtime::Engine` (`Send + Sync`),
//!   configured with async support so host imports can suspend the component.
//! - **Stores** (`store.rs`) — the **dual-store model**. A WASM `Store` is
//!   `!Send`, but jinn fires hooks from two threads: the async host thread
//!   (lifecycle hooks, LLM callbacks) and the render thread (sync render
//!   hooks). So each component that exports both kinds of hooks is
//!   instantiated in *two* stores, mirroring the old Lua system's two Lua
//!   states. Both stores read/write the same host-owned bag layer, keyed by
//!   `PluginInstanceId`.
//! - **Bags** (`bag.rs`) — the `PluginData` / `GlobalPluginData` layer ported
//!   to opaque `Vec<u8>` (postcard-blind). The host never inspects contents.
//! - **Discovery** (`discovery.rs`) — `.wasm` + sidecar `plugin.toml`.
//! - **Imports** (`imports.rs`) — host-import implementations: `emit`,
//!   `request-llm-oneshot`, `create-session`, `cancel-task`, bag accessors.
//! - **Hook firing** (`hooks.rs`) — runtime export lookup; absent exports are
//!   skipped (optional-hook semantics).
//!
//! Domain code never touches wasmtime directly; this crate sits behind the
//! existing `PluginFire` / `PluginSyncCall` / `PluginSyncHooks` trait seam in
//! `jinn-domain`.

// This crate establishes the WASM host in Phase 2; its public types are
// consumed by the wiring layer in Phase 3. Until then, allow dead code.
#![allow(dead_code)]

pub mod bag;
pub mod bindings;
pub mod discovery;
mod host_impl;
mod engine;
mod hooks;
pub mod loader;
mod imports;
mod store;


pub use bag::{GlobalBagStore, InstanceBagStore};
pub use discovery::{PluginKind, PluginMeta, discover_plugins};
pub use engine::{EngineConfig, WasmEngine};

pub use store::{StoreKind, StoreSet};
