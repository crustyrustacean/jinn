//! Service wrapper for strategy discovery.
//!
//! Wraps `Arc<dyn StrategyDiscovery>` for shared ownership across
//! the application, following the service wrapper pattern.

use std::sync::Arc;

use nullslop_context::{StrategyDiscovery, StrategyInfo};

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

#[cfg(test)]
mod tests {
    use nullslop_context::{DefaultStrategyDiscovery, StrategyDiscovery as _};

    use super::*;

    #[rstest::rstest]
    fn service_delegates_list() {
        // Given a service wrapping the default discovery.
        let service = StrategyRegistryService::new(Arc::new(DefaultStrategyDiscovery));

        // When listing strategies.
        let strategies = service.list();

        // Then the result matches the underlying discovery.
        assert_eq!(strategies.len(), DefaultStrategyDiscovery.list().len());
    }

    #[rstest::rstest]
    fn service_delegates_name() {
        // Given a service wrapping the default discovery.
        let service = StrategyRegistryService::new(Arc::new(DefaultStrategyDiscovery));

        // Then it exposes the underlying name.
        assert_eq!(service.name(), "default_strategy_discovery");
    }
}
