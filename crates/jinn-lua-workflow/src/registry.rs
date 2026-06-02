//! Lua VM registry — manages VM handles without owning any `Lua` instances.
//!
//! [`LuaRegistry`] holds [`VmHandle`] structs (channel senders + join handles),
//! which are fully `Send + Sync`. The actual `mlua::Lua` instances live inside
//! their own tokio tasks and never cross thread boundaries.

use std::collections::HashMap;

use error_stack::Report;
use serde_json::Value;
use wherror::Error;

use crate::protocol::HostRequest;

/// Error type for Lua workflow operations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct LuaError;

impl LuaError {
    /// Creates a task-failed error.
    pub fn task_failed(msg: impl Into<String>) -> Report<Self> {
        Report::new(Self).attach(msg.into())
    }

    /// Creates a script error.
    pub fn script(msg: impl Into<String>) -> Report<Self> {
        Report::new(Self).attach(msg.into())
    }
}

/// Handle to a running Lua VM task.
///
/// Does NOT own a `Lua` instance — only a channel sender for sending
/// [`HostRequest`]s and a join handle for awaiting the result.
/// Fully `Send + Sync`.
pub struct VmHandle {
    /// Channel sender for communicating with the host handler.
    host_tx: kanal::Sender<HostRequest>,
    /// Join handle for the VM's tokio task.
    join_handle: tokio::task::JoinHandle<Result<Value, Report<LuaError>>>,
}

impl std::fmt::Debug for VmHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VmHandle")
            .field("host_tx", &"kanal::Sender<HostRequest>")
            .field("join_handle", &"JoinHandle<...>")
            .finish()
    }
}

impl VmHandle {
    /// Creates a new `VmHandle`.
    pub fn new(
        host_tx: kanal::Sender<HostRequest>,
        join_handle: tokio::task::JoinHandle<Result<Value, Report<LuaError>>>,
    ) -> Self {
        Self {
            host_tx,
            join_handle,
        }
    }

    /// Returns a clone of the channel sender for sending host requests.
    pub fn host_tx(&self) -> kanal::Sender<HostRequest> {
        self.host_tx.clone()
    }

    /// Returns a reference to the join handle.
    pub fn join_handle(&self) -> &tokio::task::JoinHandle<Result<Value, Report<LuaError>>> {
        &self.join_handle
    }
}

/// Registry for Lua VM handles.
///
/// Maps plugin names to [`VmHandle`] instances. Does NOT own any `Lua` instances.
/// Fully `Send + Sync`.
#[derive(Debug)]
pub struct LuaRegistry {
    handles: HashMap<String, VmHandle>,
}

impl LuaRegistry {
    /// Creates a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            handles: HashMap::new(),
        }
    }

    /// Inserts a VM handle, replacing any existing handle with the same name.
    pub fn insert(&mut self, name: impl Into<String>, handle: VmHandle) {
        self.handles.insert(name.into(), handle);
    }

    /// Looks up a VM handle by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&VmHandle> {
        self.handles.get(name)
    }

    /// Removes and returns a VM handle by name.
    pub fn remove(&mut self, name: &str) -> Option<VmHandle> {
        self.handles.remove(name)
    }

    /// Returns all registered names in sorted order.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.handles.keys().cloned().collect();
        names.sort();
        names
    }

    /// Returns `true` if no VMs are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    /// Returns the number of registered VMs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.handles.len()
    }
}

impl Default for LuaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        reason = "test code, panics are acceptable"
    )]

    use super::*;

    /// Creates a stub `VmHandle` for testing (does not spawn a real VM).
    fn stub_handle() -> VmHandle {
        let (tx, _rx) = kanal::unbounded::<HostRequest>();
        let join_handle = tokio::spawn(async { Ok(Value::Null) });
        VmHandle::new(tx, join_handle)
    }

    #[tokio::test]
    async fn new_registry_is_empty() {
        let registry = LuaRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(registry.names().is_empty());
    }

    #[tokio::test]
    async fn insert_adds_handle() {
        let mut registry = LuaRegistry::new();
        registry.insert("test-vm", stub_handle());

        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
        assert!(registry.get("test-vm").is_some());
    }

    #[tokio::test]
    async fn get_returns_none_for_unknown() {
        let mut registry = LuaRegistry::new();
        registry.insert("known", stub_handle());

        assert!(registry.get("unknown").is_none());
    }

    #[tokio::test]
    async fn remove_takes_handle_out() {
        let mut registry = LuaRegistry::new();
        registry.insert("to-remove", stub_handle());

        let removed = registry.remove("to-remove");
        assert!(removed.is_some());
        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn remove_returns_none_for_unknown() {
        let mut registry = LuaRegistry::new();
        assert!(registry.remove("nonexistent").is_none());
    }

    #[tokio::test]
    async fn names_returns_sorted() {
        let mut registry = LuaRegistry::new();
        registry.insert("charlie", stub_handle());
        registry.insert("alpha", stub_handle());
        registry.insert("bravo", stub_handle());

        assert_eq!(registry.names(), vec!["alpha", "bravo", "charlie"]);
    }

    #[tokio::test]
    async fn insert_replaces_existing() {
        let mut registry = LuaRegistry::new();
        registry.insert("dup", stub_handle());
        registry.insert("dup", stub_handle());

        assert_eq!(registry.len(), 1);
    }
}
