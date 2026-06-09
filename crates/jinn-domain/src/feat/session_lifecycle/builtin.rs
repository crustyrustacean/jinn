//! Builtin lifecycle commands - extending session lifecycles with compiled Rust handlers.
//!
//! Provides [`LifecycleCommand`] enum that supports both shell commands (backward compatible)
//! and builtin handlers identified by [`BuiltinId`]. The serde implementation ensures existing
//! TOML configs (bare string commands) continue to work.

use error_stack::Report;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use crate::protocol::SessionId;

/// Identifies a compiled-in lifecycle handler.
///
/// Used as the discriminant in [`LifecycleCommand::Builtin`] to look up
/// a registered handler in the builtin registry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BuiltinId(pub String);

impl fmt::Display for BuiltinId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A lifecycle command - either a shell string or a builtin handler reference.
///
/// # Serde behavior
///
/// - `Shell("echo /tmp")` serializes as `"echo /tmp"` (bare string)
/// - `Builtin(BuiltinId("hello-world"))` serializes as `{ builtin = "hello-world" }`
///
/// This ensures backward compatibility with existing `jinn.toml` configs
/// that use bare strings for `setup_command` and `teardown_command`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LifecycleCommand {
    /// A shell command to execute via `$SHELL -c`.
    Shell(String),
    /// A reference to a compiled-in lifecycle handler.
    Builtin(BuiltinId),
}

impl Serialize for LifecycleCommand {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Shell(s) => serializer.serialize_str(s),
            Self::Builtin(id) => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("builtin", &id.0)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for LifecycleCommand {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, Visitor};

        /// Helper struct for deserializing the `{ builtin = "name" }` form.
        #[expect(dead_code, reason = "field accessed by serde deserialization")]
        #[derive(Deserialize)]
        struct BuiltinForm {
            builtin: String,
        }

        enum Field {
            Builtin,
        }

        impl<'de> Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct FieldVisitor;

                impl Visitor<'_> for FieldVisitor {
                    type Value = Field;

                    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                        formatter.write_str("`builtin`")
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Field, E>
                    where
                        E: de::Error,
                    {
                        match value {
                            "builtin" => Ok(Field::Builtin),
                            _ => Err(de::Error::unknown_field(value, &["builtin"])),
                        }
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct LifecycleCommandVisitor;

        impl<'de> Visitor<'de> for LifecycleCommandVisitor {
            type Value = LifecycleCommand;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a string or { builtin = \"name\" }")
            }

            fn visit_str<E>(self, value: &str) -> Result<LifecycleCommand, E>
            where
                E: de::Error,
            {
                Ok(LifecycleCommand::Shell(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<LifecycleCommand, E>
            where
                E: de::Error,
            {
                Ok(LifecycleCommand::Shell(value))
            }

            fn visit_map<A>(self, mut map: A) -> Result<LifecycleCommand, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut builtin_id: Option<String> = None;
                while let Some(key) = map.next_key::<Field>()? {
                    match key {
                        Field::Builtin => {
                            if builtin_id.is_some() {
                                return Err(de::Error::duplicate_field("builtin"));
                            }
                            builtin_id = Some(map.next_value()?);
                        }
                    }
                }
                let id = builtin_id.ok_or_else(|| de::Error::missing_field("builtin"))?;
                Ok(LifecycleCommand::Builtin(BuiltinId(id)))
            }
        }

        deserializer.deserialize_any(LifecycleCommandVisitor)
    }
}

/// Error type for builtin handler failures.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct BuiltinHandlerError;

/// A builtin lifecycle handler.
///
/// Each builtin lifecycle (e.g., bench tasks) registers a handler that provides
/// setup and teardown behavior. Setup returns a working directory path; teardown
/// performs cleanup and verification.
pub trait BuiltinHandler: Send + Sync {
    /// Returns a human-readable name for this handler, for debugging.
    fn name(&self) -> &'static str;

    /// Run setup for this builtin lifecycle.
    ///
    /// Returns the working directory path to set as the session's CWD.
    ///
    /// # Errors
    ///
    /// Returns an error if setup fails (e.g., fixture preparation fails).
    fn setup(
        &self,
        session_id: &SessionId,
        args: &[String],
    ) -> Result<PathBuf, Report<BuiltinHandlerError>>;

    /// Run teardown for this builtin lifecycle.
    ///
    /// Returns `true` if teardown succeeded, `false` if it failed.
    fn teardown(&self, session_id: &SessionId, args: &[String]) -> bool;
}

/// Registry of builtin lifecycle handlers, keyed by [`BuiltinId`].
///
/// Created empty and populated before the actor system starts. Passed to the
/// session actor via [`SessionPersistenceActorDeps`].
///
/// [`SessionPersistenceActorDeps`]: crate::feat::session::session_actor::SessionPersistenceActorDeps
#[derive(Clone, Default)]
pub struct BuiltinRegistry {
    handlers: HashMap<BuiltinId, Arc<dyn BuiltinHandler>>,
}

impl BuiltinRegistry {
    /// Creates a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a builtin handler under the given id.
    pub fn register<I>(&mut self, id: I, handler: Arc<dyn BuiltinHandler>)
    where
        I: Into<BuiltinId>,
    {
        self.handlers.insert(id.into(), handler);
    }

    /// Looks up a handler by id.
    #[must_use]
    pub fn get(&self, id: &BuiltinId) -> Option<&Arc<dyn BuiltinHandler>> {
        self.handlers.get(id)
    }

    /// Returns `true` if no handlers are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

impl std::fmt::Debug for BuiltinRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltinRegistry")
            .field("count", &self.handlers.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unreachable, reason = "test code")]

    use super::*;

    #[derive(Serialize, Deserialize)]
    struct CommandWrapper {
        cmd: LifecycleCommand,
    }

    #[derive(Deserialize)]
    struct SetupCommandWrapper {
        setup_command: LifecycleCommand,
    }

    #[test]
    fn shell_serializes_as_bare_string() {
        // Given a Shell command.
        let cmd = LifecycleCommand::Shell("echo /tmp".to_owned());

        // When serializing to TOML (through a wrapper for valid TOML document).
        let toml_str = toml::to_string(&CommandWrapper { cmd }).expect("serialize");

        // Then it's a bare string.
        assert!(toml_str.contains("echo /tmp"));
    }

    #[test]
    fn builtin_serializes_as_map() {
        // Given a Builtin command.
        let cmd = LifecycleCommand::Builtin(BuiltinId("hello-world".to_owned()));

        // When serializing to TOML (through a wrapper for valid TOML document).
        let toml_str = toml::to_string(&CommandWrapper { cmd }).expect("serialize");

        // Then it's { builtin = "hello-world" }.
        assert!(toml_str.contains("builtin"));
        assert!(toml_str.contains("hello-world"));
    }

    #[test]
    fn bare_string_deserializes_as_shell() {
        // Given a bare string TOML value (wrapped in a table for valid TOML).
        let toml_str = r#"setup_command = "echo /tmp""#;

        // When deserializing through a wrapper.
        let wrapper: SetupCommandWrapper = toml::from_str(toml_str).expect("deserialize");

        // Then it's a Shell variant.
        assert_eq!(
            wrapper.setup_command,
            LifecycleCommand::Shell("echo /tmp".to_owned())
        );
    }

    #[test]
    fn builtin_map_deserializes_as_builtin() {
        // Given a { builtin = "name" } TOML value (wrapped in a table for valid TOML).
        let toml_str = r#"setup_command = { builtin = "hello-world" }"#;

        // When deserializing through a wrapper.
        let wrapper: SetupCommandWrapper = toml::from_str(toml_str).expect("deserialize");

        // Then it's a Builtin variant.
        assert_eq!(
            wrapper.setup_command,
            LifecycleCommand::Builtin(BuiltinId("hello-world".to_owned()))
        );
    }

    #[test]
    fn roundtrip_shell() {
        // Given a Shell command.
        let original = LifecycleCommand::Shell("echo /tmp".to_owned());

        // When serializing and deserializing through a wrapper.
        let toml_str = toml::to_string(&CommandWrapper {
            cmd: original.clone(),
        })
        .expect("serialize");
        let restored: CommandWrapper = toml::from_str(&toml_str).expect("deserialize");

        // Then it matches the original.
        assert_eq!(restored.cmd, original);
    }

    #[test]
    fn roundtrip_builtin() {
        // Given a Builtin command.
        let original = LifecycleCommand::Builtin(BuiltinId("hello-world".to_owned()));

        // When serializing and deserializing through a wrapper.
        let toml_str = toml::to_string(&CommandWrapper {
            cmd: original.clone(),
        })
        .expect("serialize");
        let restored: CommandWrapper = toml::from_str(&toml_str).expect("deserialize");

        // Then it matches the original.
        assert_eq!(restored.cmd, original);
    }

    // --- Phase 7: Mutation-killing tests ---

    #[test]
    fn builtin_id_display_outputs_inner_string() {
        // Given a BuiltinId.
        let id = BuiltinId("hello-world".to_owned());

        // When displaying.
        let displayed = format!("{id}");

        // Then the inner string is shown (not empty).
        assert_eq!(displayed, "hello-world");
    }

    #[test]
    fn registry_register_and_get_roundtrip() {
        // Given an empty registry.
        let mut registry = BuiltinRegistry::new();

        // When registering a handler.
        let handler = Arc::new(MockHandler);
        registry.register(BuiltinId("test-handler".to_owned()), handler);

        // Then get returns Some.
        let result = registry.get(&BuiltinId("test-handler".to_owned()));
        assert!(result.is_some());
    }

    #[test]
    fn registry_get_returns_none_for_unknown() {
        // Given a registry with one handler.
        let mut registry = BuiltinRegistry::new();
        registry.register(BuiltinId("known".to_owned()), Arc::new(MockHandler));

        // When looking up an unknown handler.
        let result = registry.get(&BuiltinId("unknown".to_owned()));

        // Then None is returned.
        assert!(result.is_none());
    }

    #[test]
    fn registry_is_empty_true_when_no_handlers() {
        // Given an empty registry.
        let registry = BuiltinRegistry::new();

        // When checking is_empty.
        assert!(registry.is_empty());
    }

    #[test]
    fn registry_is_empty_false_after_register() {
        // Given a registry with one handler.
        let mut registry = BuiltinRegistry::new();
        registry.register(BuiltinId("test".to_owned()), Arc::new(MockHandler));

        // When checking is_empty.
        assert!(!registry.is_empty());
    }

    #[test]
    fn registry_debug_shows_count() {
        // Given a registry with two handlers.
        let mut registry = BuiltinRegistry::new();
        registry.register(BuiltinId("a".to_owned()), Arc::new(MockHandler));
        registry.register(BuiltinId("b".to_owned()), Arc::new(MockHandler));

        // When debugging.
        let debug_str = format!("{registry:?}");

        // Then the debug output contains "count" and a number.
        assert!(debug_str.contains("count"));
    }

    #[test]
    fn deserialize_invalid_field_returns_error() {
        // Given TOML with an unknown field in a builtin map.
        let toml_str = r#"setup_command = { unknown = "hello" }"#;

        // When deserializing.
        let result: Result<SetupCommandWrapper, _> = toml::from_str(toml_str);

        // Then an error is returned (triggers the `expecting` message in FieldVisitor).
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_non_string_non_map_returns_error() {
        // Given TOML where the command is a number (not a string or map).
        let toml_str = r#"setup_command = 42"#;

        // When deserializing.
        let result: Result<SetupCommandWrapper, _> = toml::from_str(toml_str);

        // Then an error is returned (triggers the `expecting` message in LifecycleCommandVisitor).
        assert!(result.is_err());
    }

    /// A minimal mock handler for testing the registry.
    struct MockHandler;

    impl BuiltinHandler for MockHandler {
        fn name(&self) -> &'static str {
            "mock"
        }

        fn setup(
            &self,
            _session_id: &SessionId,
            _args: &[String],
        ) -> Result<PathBuf, Report<BuiltinHandlerError>> {
            Ok(PathBuf::from("/tmp/mock"))
        }

        fn teardown(&self, _session_id: &SessionId, _args: &[String]) -> bool {
            true
        }
    }
}
