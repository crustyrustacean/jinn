//! Host-owned plugin state bags — opaque `Vec<u8>`.
//!
//! Ported from `jinn-plugin/src/plugin_data.rs`, but the stored value is now
//! `Vec<u8>` instead of `serde_json::Value`. The host never inspects the
//! contents; the plugin serializes/deserializes via its PDK codec (postcard by
//! default). This is the WASM realization of the proven host-owned-bag model.
//!
//! # Keying
//!
//! - Attached plugins key on `(SessionId, PluginInstanceId)` so duplicate
//!   instances of the same plugin name get isolated slots.
//! - Global plugins (no instance) key on their name.
//!
//! Both stores (async + sync) share one `InstanceBagStore` / one
//! `GlobalBagStore` via `Arc`, so a write from the async host thread is
//! visible to the render thread on its next read — same visibility contract
//! as the Lua system.

use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;
use jinn_core_types::{PluginInstanceId, SessionId};
use parking_lot::RwLock;

/// Composite key for the per-instance bag store.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BagKey {
    /// Bag scoped to a specific attached instance in a session.
    Attached(SessionId, PluginInstanceId),
    /// Bag scoped to a global plugin.
    Global(String),
}

/// Thread-safe per-instance bag store.
///
/// Keys map to opaque bytes. Cloned cheaply via `Arc`; the underlying
/// `DashMap` is shared, so the sync store and the async store observe each
/// other's writes.
#[derive(Debug, Clone)]
pub struct InstanceBagStore(Arc<DashMap<BagKey, Vec<u8>>>);

impl InstanceBagStore {
    /// Create a new empty store.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(DashMap::new()))
    }

    /// Snapshot an attached instance's bag bytes.
    #[must_use]
    pub fn get_for_session(
        &self,
        session_id: &SessionId,
        instance_id: &PluginInstanceId,
    ) -> Option<Vec<u8>> {
        self.0
            .get(&BagKey::Attached(session_id.clone(), instance_id.clone()))
            .map(|v| v.clone())
    }

    /// Replace an attached instance's bag bytes.
    pub fn set_for_session(
        &self,
        session_id: &SessionId,
        instance_id: &PluginInstanceId,
        bytes: Vec<u8>,
    ) {
        self.0.insert(
            BagKey::Attached(session_id.clone(), instance_id.clone()),
            bytes,
        );
    }

    /// Snapshot a global plugin's bag bytes.
    #[must_use]
    pub fn get(&self, plugin_name: &str) -> Option<Vec<u8>> {
        self.0
            .get(&BagKey::Global(plugin_name.to_owned()))
            .map(|v| v.clone())
    }

    /// Replace a global plugin's bag bytes.
    pub fn set(&self, plugin_name: &str, bytes: Vec<u8>) {
        self.0.insert(BagKey::Global(plugin_name.to_owned()), bytes);
    }

    /// Resolve the bag for the current instance context.
    ///
    /// Attached instances read their `(session, instance)` slot; global
    /// instances (no session) read their name slot.
    #[must_use]
    pub fn get_for_session_ctx(&self, ctx: &crate::store::InstanceCtx) -> Option<Vec<u8>> {
        match &ctx.session_id {
            Some(sid) => self.get_for_session(sid, &ctx.instance_id),
            None => self.get(&ctx.plugin_name),
        }
    }
}

impl Default for InstanceBagStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe global data store — cross-plugin, cross-instance shared state.
///
/// A string-keyed bag of opaque bytes any plugin or instance can read or
/// write. Used for cross-instance coordination (e.g. multi-judge aggregation:
/// a judge posts its verdict under a shared key; the last-to-finish reads all
/// verdicts and merges them).
///
/// Reads return a fresh snapshot; reads-modify-write sequences are race-free
/// in practice because the plugin execution thread is single-threaded within
/// a store. If jinn ever moves to multi-threaded plugin execution within one
/// store, this invariant would need atomic/CAS ops.
#[derive(Debug, Clone)]
pub struct GlobalBagStore(Arc<RwLock<HashMap<String, Vec<u8>>>>);

impl GlobalBagStore {
    /// Create a new empty global store.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(HashMap::new())))
    }

    /// Snapshot the bytes under `key`, if present.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.0.read().get(key).cloned()
    }

    /// Replace the bytes under `key`.
    pub fn set(&self, key: &str, bytes: Vec<u8>) {
        self.0.write().insert(key.to_owned(), bytes);
    }

    /// Remove the bytes under `key`, returning them if present.
    pub fn remove(&self, key: &str) -> Option<Vec<u8>> {
        self.0.write().remove(key)
    }

    /// Returns the set of keys currently in the store. Used for aggregation
    /// scans (e.g. judge reads all `verdict:*` keys).
    pub fn keys(&self) -> Vec<String> {
        self.0.read().keys().cloned().collect()
    }
}

impl Default for GlobalBagStore {
    fn default() -> Self {
        Self::new()
    }
}
