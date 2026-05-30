//! Node registry and factory trait.
//!
//! Provides a [`NodeRegistry`] that maps type names to [`NodeFactory`] instances.
//! Used to construct nodes from runtime data (type name + configuration), which
//! is a prerequisite for data-driven graph construction and scripting integration.

use std::collections::HashMap;

use derive_more::Display;
use error_stack::Report;
use wherror::Error;

use crate::node::WorkflowNode;

/// Error type for registry operations.
#[derive(Debug, Error, Display)]
pub enum RegistryError {
    /// The requested node type was not found in the registry.
    #[display("node type '{type_name}' not found in registry")]
    NotFound {
        /// The type name that was requested.
        type_name: String,
    },
    /// The factory failed to create a node.
    #[display("failed to create node type '{type_name}': {reason}")]
    CreationFailed {
        /// The type name that was being created.
        type_name: String,
        /// Description of why creation failed.
        reason: String,
    },
}

/// Factory trait for creating nodes from configuration.
///
/// Implement this trait to provide a named constructor for a node type.
/// Register instances with [`NodeRegistry::register`].
pub trait NodeFactory: Send + Sync {
    /// Creates a new node from the given configuration.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::CreationFailed`] if the configuration is invalid.
    fn create(
        &self,
        config: serde_json::Value,
    ) -> Result<Box<dyn WorkflowNode>, Report<RegistryError>>;
}

/// A registry of node factories, mapping type names to factories.
///
/// Create with [`NodeRegistry::new`], register factories with
/// [`NodeRegistry::register`], and create nodes with [`NodeRegistry::create`].
pub struct NodeRegistry {
    /// Map of type names to their factories.
    factories: HashMap<String, Box<dyn NodeFactory>>,
}

impl NodeRegistry {
    /// Creates a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Registers a factory for a node type.
    pub fn register<S>(&mut self, type_name: S, factory: Box<dyn NodeFactory>)
    where
        S: Into<String>,
    {
        self.factories.insert(type_name.into(), factory);
    }

    /// Returns the factory for a given type name.
    pub fn get(&self, type_name: &str) -> Option<&dyn NodeFactory> {
        self.factories.get(type_name).map(Box::as_ref)
    }

    /// Creates a node from the registry.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::NotFound`] if the type name is not registered.
    /// Returns [`RegistryError::CreationFailed`] if the factory fails.
    pub fn create(
        &self,
        type_name: &str,
        config: serde_json::Value,
    ) -> Result<Box<dyn WorkflowNode>, Report<RegistryError>> {
        let factory = self.factories.get(type_name).ok_or_else(|| {
            Report::new(RegistryError::NotFound {
                type_name: type_name.to_owned(),
            })
        })?;
        factory.create(config)
    }
}

impl Default for NodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        dead_code,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::unnecessary_literal_bound,
        reason = "test code"
    )]
    use super::*;
    use crate::node::NodeContext;
    use error_stack::Report;
    use std::time::Duration;

    /// A test context for node execution.
    struct TestContext;
    impl NodeContext for TestContext {}

    /// A simple delay factory for testing.
    struct PassthroughFactory;

    impl NodeFactory for PassthroughFactory {
        fn create(
            &self,
            _config: serde_json::Value,
        ) -> Result<Box<dyn WorkflowNode>, Report<RegistryError>> {
            Ok(Box::new(crate::node::delay::DelayNode::passthrough(
                Duration::from_millis(1),
            )))
        }
    }

    #[test]
    fn new_registry_is_empty() {
        let registry = NodeRegistry::new();
        assert!(registry.get("anything").is_none());
    }

    #[test]
    fn register_and_retrieve_factory() {
        let mut registry = NodeRegistry::new();
        registry.register("passthrough", Box::new(PassthroughFactory));
        assert!(registry.get("passthrough").is_some());
        assert!(registry.get("unknown").is_none());
    }

    #[test]
    fn create_returns_node_for_registered_type() {
        let mut registry = NodeRegistry::new();
        registry.register("passthrough", Box::new(PassthroughFactory));

        let node = registry
            .create("passthrough", serde_json::json!({}))
            .expect("create");

        assert_eq!(node.name(), "delay");
    }

    #[test]
    fn create_returns_not_found_for_unknown_type() {
        let registry = NodeRegistry::new();
        let result = registry.create("unknown", serde_json::json!({}));
        assert!(matches!(
            result,
            Err(e) if matches!(e.current_context(), RegistryError::NotFound { type_name } if type_name == "unknown")
        ));
    }

    #[test]
    fn create_returns_creation_failed_for_bad_config() {
        // A factory that requires a "name" field.
        struct StrictFactory;
        impl NodeFactory for StrictFactory {
            fn create(
                &self,
                config: serde_json::Value,
            ) -> Result<Box<dyn WorkflowNode>, Report<RegistryError>> {
                let _name = config.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                    Report::new(RegistryError::CreationFailed {
                        type_name: "strict".to_owned(),
                        reason: "missing 'name' field".to_owned(),
                    })
                })?;
                Ok(Box::new(crate::node::delay::DelayNode::passthrough(
                    Duration::from_millis(1),
                )))
            }
        }

        let mut registry = NodeRegistry::new();
        registry.register("strict", Box::new(StrictFactory));

        let result = registry.create("strict", serde_json::json!({}));
        assert!(matches!(
            result,
            Err(e) if matches!(e.current_context(), RegistryError::CreationFailed { .. })
        ));
    }
}
