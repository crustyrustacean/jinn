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
// Test fabric: a real trouper system, no kernel Services.
// ---------------------------------------------------------------------------

/// A minimal test harness over a real trouper `ActorSystem`, with the
/// spawn/send surface slice crates actually use. Slice-crate tests spin
/// this up instead of the kernel's `Services`, which they must not
/// depend on.
///
/// Not `Default` — construction creates an actor system on the ambient
/// runtime, which is a deliberate, visible action (`new`), not a value
/// default.
pub struct TestFabric {
    /// The trouper system.
    system: trouper::system::ActorSystem,
}

impl TestFabric {
    /// The trouper system, for spawning actors under test.
    #[must_use]
    pub fn system(&self) -> &trouper::system::ActorSystem {
        &self.system
    }

    /// Creates a fresh trouper system.
    #[expect(
        clippy::new_without_default,
        reason = "construction creates an actor system on the ambient runtime — an action, not a value default"
    )]
    pub fn new() -> Self {
        let system = trouper::system::ActorSystem::new(trouper::system::SystemConfig::production());
        Self { system }
    }

    /// Sends a typed message to the trouper `topic`.
    pub async fn send_to_topic<M: trouper::schema::Schema + serde::Serialize>(
        &self,
        msg: &M,
        topic: &trouper::topics::Topic,
    ) {
        let payload = serde_json::to_value(msg).unwrap_or(serde_json::Value::Null);
        let event = trouper::envelope::Event::new(M::schema_id(), payload);
        let _ = self
            .system
            .send(self.system.envelope_to_topic(event, topic.clone()))
            .await;
    }
}
