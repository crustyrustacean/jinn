//! Service wrapper for strategy discovery.
//!
//! Wraps `Arc<dyn StrategyDiscovery>` for shared ownership across
//! the application, following the service wrapper pattern.

use std::sync::Arc;

use nsslice_context_protocol::{StrategyDiscovery, StrategyInfo};

/// Service wrapper around a [`StrategyDiscovery`] implementation.
///
/// Provides shared ownership and cheap cloning for use throughout
/// the application via the [`Services`](crate::Services) container.
#[derive(Clone)]
pub struct StrategyRegistryService {
    /// The underlying discovery implementation.
    svc: Arc<dyn StrategyDiscovery>,
}

impl std::fmt::Debug for StrategyRegistryService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StrategyRegistryService")
            .field("name", &self.svc.name())
            .finish()
    }
}

impl StrategyRegistryService {
    /// Creates a new strategy registry service wrapping the given discovery.
    #[must_use]
    pub fn new(discovery: Arc<dyn StrategyDiscovery>) -> Self {
        Self { svc: discovery }
    }

    /// Returns all available strategies from the underlying discovery.
    #[must_use]
    pub fn list(&self) -> Vec<StrategyInfo> {
        self.svc.list()
    }

    /// Returns the name of the underlying discovery, for debugging.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.svc.name()
    }
}
