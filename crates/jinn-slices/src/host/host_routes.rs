//! Bridge route staging: forward/reverse declarations with an
//! activation-time conflict gate.
//!
//! Routes are part of a slice's manifest. A slice declares at
//! activation which crossing messages it owns in each direction; the
//! kernel's bridge drain wiring consumes the staged set
//! ([`StagedRoutes`]) to subscribe the bus actors. A message may cross
//! in at most one direction — declaring both panics immediately (in
//! any profile), preserving the bridge's no-feedback-loop guarantee at
//! the moment of the wiring mistake instead of at first publish.

use std::collections::HashMap;

use trouper::schema::Schema;
use trouper::schema::SchemaDef;
use trouper::types::SchemaId;
use trouper::types::Topic;

/// A message that can cross kameo → trouper (forward).
pub trait ForwardMessage: Schema + serde::Serialize + Clone + Send + 'static {}

/// A message that can cross trouper → kameo (reverse).
pub trait ReverseMessage:
    Schema + serde::Serialize + serde::de::DeserializeOwned + Clone + Send + 'static
{
}

/// Blanket impls: any qualifying message crosses automatically.
impl<T> ForwardMessage for T where T: Schema + serde::Serialize + Clone + Send + 'static {}
impl<T> ReverseMessage for T where
    T: Schema + serde::Serialize + serde::de::DeserializeOwned + Clone + Send + 'static
{
}

/// One staged route: the message's schema id, its display name, the
/// topic it crosses on, and the direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteEntry {
    /// The message's schema id (the bridge's crossing identity).
    pub schema_id: SchemaId,
    /// Human-readable message name for diagnostics.
    pub name: &'static str,
    /// The trouper topic the message crosses on.
    pub topic: Topic,
    /// The direction of travel.
    pub direction: Direction,
}

/// Which way a message crosses the fabrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// kameo bus → trouper topic.
    Forward,
    /// trouper topic → kameo bus.
    Reverse,
}

impl std::fmt::Display for Direction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Direction::Forward => f.write_str("forward"),
            Direction::Reverse => f.write_str("reverse"),
        }
    }
}

/// A route declaration rejected at activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteConflict {
    /// The schema id declared twice.
    pub schema_id: SchemaId,
    /// The already-registered direction.
    pub existing: Direction,
    /// The newly-declared direction.
    pub attempted: Direction,
}

impl std::fmt::Display for RouteConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "schema {} already registered {}; dual-direction registration creates a feedback loop",
            self.schema_id, self.existing
        )
    }
}

impl std::error::Error for RouteConflict {}

/// Registry of staged routes for one activation (or the union across
/// activations — composition decides the lifetime).
#[derive(Debug, Default)]
pub struct RouteRegistry {
    by_schema: HashMap<SchemaId, RouteEntry>,
}

impl RouteRegistry {
    /// Stages a forward route for `M` on `topic`.
    ///
    /// # Panics
    ///
    /// Panics if `M`'s schema id is already registered in either
    /// direction — dual-direction registration is a feedback loop.
    pub fn forward<M: ForwardMessage, S>(&mut self, topic: Topic, schema: S)
    where
        S: FnOnce() -> SchemaDef,
    {
        self.insert::<M, S>(topic, Direction::Forward, schema);
    }

    /// Stages a reverse route for `M` on `topic`.
    ///
    /// # Panics
    ///
    /// Panics if `M`'s schema id is already registered in either
    /// direction.
    pub fn reverse<M: ReverseMessage, S>(&mut self, topic: Topic, schema: S)
    where
        S: FnOnce() -> SchemaDef,
    {
        self.insert::<M, S>(topic, Direction::Reverse, schema);
    }

    /// Shared insert + conflict gate.
    #[expect(
        clippy::panic,
        reason = "dual-direction registration is a wiring bug caught at activation, by construction"
    )]
    fn insert<M: Schema, S>(&mut self, topic: Topic, direction: Direction, schema: S)
    where
        S: FnOnce() -> SchemaDef,
    {
        let schema_id = M::schema_id();
        if let Some(existing) = self.by_schema.get(&schema_id) {
            panic!(
                "{}",
                RouteConflict {
                    schema_id,
                    existing: existing.direction,
                    attempted: direction,
                }
            );
        }
        let name = schema().name;
        self.by_schema.insert(
            schema_id.clone(),
            RouteEntry {
                schema_id,
                name: Box::leak(name.clone().into_boxed_str()),
                topic,
                direction,
            },
        );
    }

    /// All staged routes in insertion order.
    #[must_use]
    pub fn entries(&self) -> Vec<RouteEntry> {
        let mut entries: Vec<RouteEntry> = self.by_schema.values().cloned().collect();
        entries.sort_by_key(|entry| entry.schema_id.to_string());
        entries
    }

    /// The staged route for `M`, if declared.
    #[must_use]
    pub fn entry<M: Schema>(&self) -> Option<&RouteEntry> {
        self.by_schema.get(&M::schema_id())
    }
}

/// The routes a slice staged, handed to composition's bridge wiring.
#[derive(Debug, Default)]
pub struct StagedRoutes {
    registry: RouteRegistry,
}

impl StagedRoutes {
    /// Wraps a registry into the staged handle.
    #[must_use]
    pub fn from_registry(registry: RouteRegistry) -> Self {
        Self { registry }
    }

    /// All staged routes.
    #[must_use]
    pub fn entries(&self) -> Vec<RouteEntry> {
        self.registry.entries()
    }
}

#[cfg(test)]
mod tests {
    use super::Direction;
    use super::RouteRegistry;
    use trouper::schema::Schema;
    use trouper::schema::SchemaDef;

    fn def(name: &str) -> SchemaDef {
        SchemaDef {
            name: name.to_owned(),
            version: 1,
            kind: trouper::schema::SchemaKind::Event,
            fields: vec![],
            description: None,
        }
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct TestMsg {
        value: u8,
    }

    impl trouper::schema::Schema for TestMsg {
        fn schema_def() -> SchemaDef {
            def("TestMsg")
        }
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct OtherMsg {
        value: u8,
    }

    impl trouper::schema::Schema for OtherMsg {
        fn schema_def() -> SchemaDef {
            def("OtherMsg")
        }
    }

    fn topic(name: &str) -> trouper::types::Topic {
        trouper::types::Topic::new(name)
    }

    #[rstest::rstest]
    #[test]
    fn staged_forward_route_records_schema_and_topic() {
        // Given an empty route registry.
        let mut routes = RouteRegistry::default();

        // When staging a forward route.
        routes.forward::<TestMsg, _>(topic("jinn.test"), || def("TestMsg"));

        // Then the entry records the direction, topic, and schema id.
        let entry = routes.entry::<TestMsg>().expect("staged");
        assert_eq!(entry.direction, Direction::Forward);
        assert_eq!(entry.topic, topic("jinn.test"));
        assert_eq!(entry.schema_id, TestMsg::schema_id());
    }

    #[rstest::rstest]
    #[test]
    #[should_panic(expected = "already registered")]
    fn dual_direction_registration_panics() {
        // Given a registry with TestMsg staged forward.
        let mut routes = RouteRegistry::default();
        routes.forward::<TestMsg, _>(topic("jinn.test"), || def("TestMsg"));

        // When staging the same schema in reverse.
        routes.reverse::<TestMsg, _>(topic("jinn.test"), || def("TestMsg"));

        // Then registration panics — a feedback loop by construction.
    }

    #[rstest::rstest]
    #[test]
    fn distinct_schemas_stage_in_both_directions() {
        // Given a registry with one forward and one reverse route.
        let mut routes = RouteRegistry::default();
        routes.forward::<TestMsg, _>(topic("jinn.a"), || def("TestMsg"));
        routes.reverse::<OtherMsg, _>(topic("jinn.b"), || def("OtherMsg"));

        // Then both entries coexist.
        assert_eq!(routes.entries().len(), 2);
        assert!(routes.entry::<TestMsg>().is_some());
        assert!(routes.entry::<OtherMsg>().is_some());
    }
}
