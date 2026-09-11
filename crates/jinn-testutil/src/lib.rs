//! Shared test utilities for jinn TUI rendering tests.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

/// Creates a test terminal and full-area rect for the given dimensions.
///
/// # Panics
///
/// Panics if `Terminal::new` fails, which should only happen with zero-sized dimensions.
pub fn setup_term(width: u16, height: u16) -> (Terminal<TestBackend>, Rect) {
    let backend = TestBackend::new(width, height);
    let terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, width, height);
    (terminal, area)
}

/// Extracts a single row from a ratatui buffer as a `String`.
pub fn buffer_row(buffer: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
    (0..width)
        .filter_map(|x| buffer.cell((x, y)).map(ratatui::buffer::Cell::symbol))
        .collect()
}

/// Extracts all rows from a ratatui buffer as `String`s.
pub fn buffer_rows(buffer: &ratatui::buffer::Buffer, width: u16, height: u16) -> Vec<String> {
    (0..height).map(|y| buffer_row(buffer, y, width)).collect()
}

// ---------------------------------------------------------------------------
// Test fabric: a real kameo bus + trouper system, no kernel Services.
// ---------------------------------------------------------------------------

/// A minimal two-fabric test harness: a real kameo message bus and a
/// real trouper `ActorSystem`, with the publish/subscribe surface slice
/// crates actually use. Slice-crate tests spin this up instead of the
/// kernel's `Services`, which they must not depend on.
///
/// Not `Default` — construction spawns actors on the ambient runtime,
/// which is a deliberate, visible action (`new`), not a value default.
#[allow(
    clippy::new_without_default,
    reason = "spawning a bus + actor system is an action, not a value default"
)]
pub struct TestFabric {
    /// The kameo bus actor ref.
    bus: kameo::actor::ActorRef<kameo_actors::message_bus::MessageBus>,
    /// The trouper system.
    system: std::sync::Arc<trouper::system::ActorSystem>,
}

impl TestFabric {
    /// The trouper system, for spawning actors under test.
    #[must_use]
    pub fn system(&self) -> &std::sync::Arc<trouper::system::ActorSystem> {
        &self.system
    }

    /// Spawns a fresh bus + trouper system.
    pub fn new() -> Self {
        let bus = kameo::actor::Spawn::spawn(kameo_actors::message_bus::MessageBus::new(
            kameo_actors::DeliveryStrategy::BestEffort,
        ));
        let system = std::sync::Arc::new(trouper::system::ActorSystem::new(
            trouper::system::SystemConfig::production(),
        ));
        Self { bus, system }
    }

    /// Publishes a typed message to all registered recipients.
    pub async fn publish<M: jinn_slices::BusMessage>(&self, msg: M) {
        let _ = self.bus.tell(kameo_actors::message_bus::Publish(msg)).await;
    }

    /// Registers a recipient for `M` on the bus.
    pub async fn register<M: jinn_slices::BusMessage>(
        &self,
        recipient: kameo::actor::Recipient<M>,
    ) {
        let _ = self
            .bus
            .ask(kameo_actors::message_bus::Register(recipient))
            .await;
    }

    /// Sends a typed message to the trouper `topic`.
    pub async fn send_to_topic<M: trouper::schema::Schema + serde::Serialize>(
        &self,
        msg: &M,
        topic: &trouper::types::Topic,
    ) {
        let payload = serde_json::to_value(msg).unwrap_or(serde_json::Value::Null);
        let event = trouper::envelope::Event::new(M::schema_id(), payload);
        let _ = self
            .system
            .send(self.system.envelope_to_topic(event, topic.clone()))
            .await;
    }
}
