//! Dashboard - displays registered actors with lifecycle and service status.
//!
//! The dashboard is a full-screen tab showing one line per actor in the
//! system. Each entry has three independent pieces of information:
//!
//! - **Name + description** — the actor's identity and what it does.
//! - **Lifecycle** — generic `Starting` / `Running` / `Dead`, driven by the
//!   existing bus events (`ActorStarting`, `ActorStarted`,
//!   `ActorShutdownCompleted`).
//! - **Status message** — an optional free-form third column, driven by
//!   [`ServiceStatusUpdate`] events published by whichever feature owns the
//!   service.
//!
//! [`DashboardCanvasActor`](canvas_actor::DashboardCanvasActor) owns the
//! dashboard's slice cell (registered under `dashboard:status` in the
//! [`Slices`](crate::common::slices::Slices) facade). It is a generic
//! sink: it receives the lifecycle events and
//! [`ServiceStatusUpdate`] events published by whichever feature owns a
//! service (bridged onto the topics by the kameo→trouper bridge), plus
//! [`DashboardNav`] for keyboard navigation — with no knowledge of any
//! individual feature.
pub mod canvas_actor;
pub mod key_routes;
pub mod nav;
pub mod view;

pub use canvas_actor::DashboardCanvasActor;
pub use key_routes::attach_dashboard_rows;
pub use key_routes::dashboard_scope;
pub use nav::DashboardNav;
use std::collections::HashMap;
pub use view::DashboardView;

/// Activates the dashboard slice: mints the cell, spawns the canvas
/// actor (subscribe is the readiness point, so no lifecycle event from
/// subsequently spawned actors is missed), attaches the route rows,
/// registers the view, and declares the tab.
///
/// One call from composition (launch/wiring) is the slice's entire
/// integration surface; commenting it out removes the slice with no
/// other edits.
///
/// # Errors
///
/// Returns the view/slot pairing error if the view cannot resolve its
/// cell — a wiring bug that must abort launch, not render blank.
pub fn activate(services: &mut crate::Services) -> Result<(), jinn_slices::view::ViewSlotError> {
    // Mint the cell: the one write handle goes into the canvas actor;
    // renderer and intent router resolve read handles only.
    let cell = services
        .slices
        .register(dashboard_slot(), DashboardState::new())
        .map_err(|_taken| jinn_slices::view::ViewSlotError {
            key: dashboard_slot(),
            reason: jinn_slices::view::ViewSlotErrorReason::Unregistered,
        })?;

    // Spawn FIRST — the dashboard must be subscribed to its topics
    // before any other actor fires lifecycle events. `subscribe`
    // registers the topic cursors synchronously, so events published
    // after this point cannot be missed, leaving no entries stuck on
    // "Starting". The bridge (spawned earlier in wiring) feeds the
    // topics from the kameo bus.
    canvas_actor::DashboardCanvasActor::spawn(&services.trouper_system, &cell);

    // Route rows + view + tab declaration.
    attach_dashboard_rows(&services.key_routes);
    services
        .viewport
        .register(DashboardView::new(), &services.slices)?;
    services
        .slices
        .register_tab_scope(dashboard_scope(), dashboard_slot());
    Ok(())
}

/// The dashboard slice's slot in the [`Slices`](crate::common::slices::Slices)
/// facade.
///
/// Canonical key shared by actor wiring (which mints the cell), the
/// renderer (which resolves a read handle), and tests.
pub fn dashboard_slot() -> crate::common::slices::SlotKey {
    crate::common::slices::SlotKey::builtin("dashboard", "status")
}

use crate::common::AppUiRegistry;

/// Generic actor lifecycle, applicable to every actor in the system.
///
/// Re-exported from `jinn-core-types` (where the type now lives) so the
/// dashboard's historical import path keeps working during extraction.
pub use jinn_core_types::ActorLifecycle;

/// A service's status for the dashboard, published by the owning feature.
///
/// Generic projection onto a dashboard row: `lifecycle: None` leaves the
/// row's lifecycle untouched (it is driven by the actor-lifecycle events),
/// and `description: None` preserves any existing description. Features
/// translate their service-specific state into this event so the dashboard
/// never needs to know a feature exists.
///
/// This is a bridge-crossing type: the kameo→trouper bridge serializes it
/// onto `jinn.fabric` under its [`Schema`] contract, so the canvas actor's
/// topic subscription can decode it — hence the serde derives.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceStatusUpdate {
    /// The dashboard row name (the key the owning feature publishes under,
    /// e.g. its `spawn_tracked!`/actor name).
    pub name: String,
    /// New row description; `None` preserves the existing one.
    pub description: Option<String>,
    /// New lifecycle; `None` leaves the row's lifecycle untouched.
    pub lifecycle: Option<ActorLifecycle>,
    /// Free-form status message for the third column.
    pub status_message: Option<String>,
}

impl crate::common::bus::BusMessage for ServiceStatusUpdate {}

/// A single actor's display data in the dashboard.
#[derive(Debug, Clone)]
pub struct DashboardEntry {
    /// The actor's display name (also its unique key).
    pub name: String,
    /// A short description of what the actor does.
    pub description: Option<String>,
    /// The actor's current lifecycle phase.
    pub lifecycle: ActorLifecycle,
    /// Free-form third column; the owning feature writes its connection or
    /// resolution status here via `ServiceStatusUpdate`.
    pub status_message: Option<String>,
}

/// Tracks the status of all actors for dashboard display.
///
/// Owned by [`DashboardCanvasActor`](canvas_actor::DashboardCanvasActor) via
/// `frontend.dashboard`. The actor owns this field.
#[derive(Debug, Clone, Default)]
pub struct DashboardState {
    /// Actor name → entry data.
    actors: HashMap<String, DashboardEntry>,
    /// Insertion-order keys for stable display.
    order: Vec<String>,
    /// Index of the currently selected actor entry.
    selected_index: usize,
    /// Vertical scroll offset in visual lines.
    scroll_offset: u16,
}

impl DashboardState {
    /// Create an empty dashboard state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the index of the currently selected actor entry.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    /// Returns the current vertical scroll offset in visual lines.
    #[must_use]
    pub fn scroll_offset(&self) -> u16 {
        self.scroll_offset
    }

    /// Clamps `scroll_offset` so the selected entry is always visible within
    /// a viewport of `viewport_height` rows.
    pub fn clamp_scroll(&mut self, viewport_height: u16) {
        if viewport_height == 0 {
            return;
        }
        let count = self.order.len() as u16;
        if count == 0 {
            self.scroll_offset = 0;
            return;
        }
        let sel = u16::try_from(self.selected_index).unwrap_or(u16::MAX);
        let bottom = self
            .scroll_offset
            .saturating_add(viewport_height)
            .saturating_sub(1);
        match sel {
            s if s < self.scroll_offset => {
                self.scroll_offset = s;
            }
            s if s > bottom => {
                self.scroll_offset = s.saturating_sub(viewport_height).saturating_add(1);
            }
            _ => {}
        }
        let max_offset = count.saturating_sub(viewport_height);
        self.scroll_offset = self.scroll_offset.min(max_offset);
    }

    /// Returns all tracked actors in insertion order.
    #[must_use]
    pub fn actors(&self) -> Vec<&DashboardEntry> {
        self.order
            .iter()
            .filter_map(|name| self.actors.get(name))
            .collect()
    }

    /// Moves the selection to the next actor entry.
    ///
    /// Clamps at the last entry - does nothing if already at the end.
    pub fn select_next(&mut self) {
        let count = self.order.len();
        if count > 0 && self.selected_index < count - 1 {
            self.selected_index += 1;
        }
    }

    /// Moves the selection to the previous actor entry.
    ///
    /// Clamps at the first entry - does nothing if already at the beginning.
    pub fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Moves the selection to the first actor entry.
    ///
    /// No-op if there are no actors.
    pub fn select_first(&mut self) {
        if !self.order.is_empty() {
            self.selected_index = 0;
        }
    }

    /// Moves the selection to the last actor entry.
    ///
    /// No-op if there are no actors.
    pub fn select_last(&mut self) {
        if !self.order.is_empty() {
            self.selected_index = self.order.len() - 1;
        }
    }

    /// Record that an actor is in (or has returned to) the startup phase.
    ///
    /// If the actor is new it is appended to the display order. Existing
    /// entries keep their description unless a new one is supplied.
    /// Resets the grid to empty — no actors, selection at top, scroll 0.
    pub fn clear(&mut self) {
        self.actors.clear();
        self.order.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Record that an actor is in (or has returned to) the startup phase.
    ///
    /// If the actor is new it is appended to the display order. Existing
    /// entries keep their description unless a new one is supplied.
    pub fn mark_starting<S>(&mut self, name: S, description: Option<String>)
    where
        S: AsRef<str>,
    {
        self.upsert(name, description, ActorLifecycle::Starting);
    }

    /// Record that an actor has finished starting and is running.
    pub fn mark_running<S>(&mut self, name: S, description: Option<String>)
    where
        S: AsRef<str>,
    {
        self.upsert(name, description, ActorLifecycle::Running);
    }

    /// Record that an actor has shut down (intentionally or via crash).
    pub fn mark_dead<S>(&mut self, name: S, description: Option<String>)
    where
        S: AsRef<str>,
    {
        self.upsert(name, description, ActorLifecycle::Dead);
    }

    /// Update only the free-form status message for an actor, leaving its
    /// lifecycle untouched.
    ///
    /// Creates the entry (as `Starting`) if it does not already exist, so the
    /// gateway can report a connection status before the corresponding
    /// `ActorStarting` bus event arrives.
    pub fn set_status_message<S>(&mut self, name: S, message: Option<String>)
    where
        S: AsRef<str>,
    {
        let name = name.as_ref();
        if !self.actors.contains_key(name) {
            self.order.push(name.to_owned());
            self.actors.insert(
                name.to_owned(),
                DashboardEntry {
                    name: name.to_owned(),
                    description: None,
                    lifecycle: ActorLifecycle::Starting,
                    status_message: message,
                },
            );
            return;
        }
        if let Some(entry) = self.actors.get_mut(name) {
            entry.status_message = message;
        }
    }

    /// Insert-or-update helper applying a new lifecycle and optional
    /// description. Does not touch `status_message` on existing entries.
    fn upsert<S>(&mut self, name: S, description: Option<String>, lifecycle: ActorLifecycle)
    where
        S: AsRef<str>,
    {
        let name = name.as_ref();
        let is_new = !self.actors.contains_key(name);
        if is_new {
            self.order.push(name.to_owned());
            self.actors.insert(
                name.to_owned(),
                DashboardEntry {
                    name: name.to_owned(),
                    description,
                    lifecycle,
                    status_message: None,
                },
            );
            return;
        }
        if let Some(entry) = self.actors.get_mut(name) {
            entry.lifecycle = lifecycle;
            if description.is_some() {
                entry.description = description;
            }
        }
    }
}

/// Register dashboard UI element.
///
/// No-op placeholder until the `DashboardElement` renderer lands in a later
/// phase; kept so callers (`actor_wiring` / feature registration) can wire it
/// without a second touch.
pub fn register(_registry: &mut AppUiRegistry) {}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;

    fn entry<'a>(state: &'a DashboardState, name: &str) -> &'a DashboardEntry {
        state.actors.get(name).expect("entry should exist")
    }

    #[rstest::rstest]
    fn mark_starting_creates_entry_with_starting_lifecycle() {
        // Given an empty dashboard.
        let mut state = DashboardState::new();

        // When marking an actor as starting.
        state.mark_starting("echo", None);

        // Then the entry exists with Starting lifecycle.
        assert_eq!(entry(&state, "echo").lifecycle, ActorLifecycle::Starting);
    }

    #[rstest::rstest]
    fn mark_running_creates_entry_with_running_lifecycle() {
        // Given an empty dashboard.
        let mut state = DashboardState::new();

        // When marking an actor as running directly.
        state.mark_running("echo", None);

        // Then the entry exists with Running lifecycle.
        assert_eq!(entry(&state, "echo").lifecycle, ActorLifecycle::Running);
    }

    #[rstest::rstest]
    fn mark_running_transitions_existing_starting_to_running() {
        // Given a dashboard with a starting actor.
        let mut state = DashboardState::new();
        state.mark_starting("echo", None);

        // When marking the same actor as running.
        state.mark_running("echo", None);

        // Then the lifecycle transitions to Running.
        assert_eq!(entry(&state, "echo").lifecycle, ActorLifecycle::Running);
    }

    #[rstest::rstest]
    fn mark_dead_transitions_to_dead() {
        // Given a dashboard with a running actor.
        let mut state = DashboardState::new();
        state.mark_running("echo", None);

        // When marking the actor as dead.
        state.mark_dead("echo", None);

        // Then the lifecycle transitions to Dead.
        assert_eq!(entry(&state, "echo").lifecycle, ActorLifecycle::Dead);
    }

    #[rstest::rstest]
    fn description_is_set_on_creation() {
        // Given an empty dashboard.
        let mut state = DashboardState::new();

        // When marking an actor as starting with a description.
        state.mark_starting("echo", Some("Echoes messages".to_owned()));

        // Then the description is stored.
        assert_eq!(
            entry(&state, "echo").description.as_deref(),
            Some("Echoes messages")
        );
    }

    #[rstest::rstest]
    fn description_is_updated_when_supplied() {
        // Given a dashboard with an actor (no description).
        let mut state = DashboardState::new();
        state.mark_starting("echo", None);

        // When marking running with a description.
        state.mark_running("echo", Some("Echoes messages".to_owned()));

        // Then the description is updated.
        assert_eq!(
            entry(&state, "echo").description.as_deref(),
            Some("Echoes messages")
        );
    }

    #[rstest::rstest]
    fn description_is_preserved_when_not_supplied() {
        // Given a dashboard with an actor that has a description.
        let mut state = DashboardState::new();
        state.mark_starting("echo", Some("Echoes messages".to_owned()));

        // When marking running without supplying a description.
        state.mark_running("echo", None);

        // Then the description is preserved (not overwritten with None).
        assert_eq!(
            entry(&state, "echo").description.as_deref(),
            Some("Echoes messages")
        );
    }

    #[rstest::rstest]
    fn set_status_message_sets_message_on_existing_entry() {
        // Given a dashboard with a running actor.
        let mut state = DashboardState::new();
        state.mark_running("some-service", None);

        // When setting a status message.
        state.set_status_message("some-service", Some("Connected".to_owned()));

        // Then the status message is set.
        assert_eq!(
            entry(&state, "some-service").status_message.as_deref(),
            Some("Connected")
        );
        // And the lifecycle is unchanged.
        assert_eq!(
            entry(&state, "some-service").lifecycle,
            ActorLifecycle::Running
        );
    }

    #[rstest::rstest]
    fn set_status_message_creates_entry_if_missing() {
        // Given an empty dashboard.
        let mut state = DashboardState::new();

        // When setting a status message for a new actor.
        state.set_status_message("some-service", Some("Connecting".to_owned()));

        // Then the entry is created with the message.
        assert_eq!(
            entry(&state, "some-service").status_message.as_deref(),
            Some("Connecting")
        );
        // And defaults to Starting lifecycle.
        assert_eq!(
            entry(&state, "some-service").lifecycle,
            ActorLifecycle::Starting
        );
    }

    #[rstest::rstest]
    fn select_next_increments_index() {
        // Given 3 actors with selection at index 0.
        let mut state = DashboardState::new();
        state.mark_starting("a", None);
        state.mark_starting("b", None);
        state.mark_starting("c", None);

        // When selecting next.
        state.select_next();

        // Then the selected index is 1.
        assert_eq!(state.selected_index(), 1);
    }

    #[rstest::rstest]
    fn select_next_clamps_at_last() {
        // Given 3 actors with selection at index 2.
        let mut state = DashboardState::new();
        state.mark_starting("a", None);
        state.mark_starting("b", None);
        state.mark_starting("c", None);
        state.select_next();
        state.select_next();

        // When selecting next again.
        state.select_next();

        // Then the index stays at 2.
        assert_eq!(state.selected_index(), 2);
    }

    #[rstest::rstest]
    fn select_prev_clamps_at_zero() {
        // Given 2 actors with selection at index 0.
        let mut state = DashboardState::new();
        state.mark_starting("a", None);
        state.mark_starting("b", None);

        // When selecting previous.
        state.select_prev();

        // Then the index stays at 0.
        assert_eq!(state.selected_index(), 0);
    }

    #[rstest::rstest]
    fn select_first_goes_to_index_zero() {
        // Given 3 actors with selection at index 2.
        let mut state = DashboardState::new();
        state.mark_starting("a", None);
        state.mark_starting("b", None);
        state.mark_starting("c", None);
        state.select_last();

        // When selecting first.
        state.select_first();

        // Then the selected index is 0.
        assert_eq!(state.selected_index(), 0);
    }

    #[rstest::rstest]
    fn select_last_goes_to_last_index() {
        // Given 3 actors with selection at index 0.
        let mut state = DashboardState::new();
        state.mark_starting("a", None);
        state.mark_starting("b", None);
        state.mark_starting("c", None);

        // When selecting last.
        state.select_last();

        // Then the selected index is 2.
        assert_eq!(state.selected_index(), 2);
    }

    #[rstest::rstest]
    fn select_next_noop_with_no_actors() {
        // Given an empty dashboard.
        let mut state = DashboardState::new();

        // When selecting next.
        state.select_next();

        // Then the index stays at 0.
        assert_eq!(state.selected_index(), 0);
    }

    #[rstest::rstest]
    fn actors_returns_insertion_order() {
        // Given a dashboard populated out of order.
        let mut state = DashboardState::new();
        state.mark_starting("c", None);
        state.mark_starting("a", None);
        state.mark_starting("b", None);

        // When querying actors.
        let names: Vec<&str> = state.actors().iter().map(|e| e.name.as_str()).collect();

        // Then the order matches insertion (c, a, b).
        assert_eq!(names, vec!["c", "a", "b"]);
    }

    #[rstest::rstest]
    fn actors_empty_returns_empty_vec() {
        // Given an empty dashboard.
        let state = DashboardState::new();

        // When querying actors.
        let actors = state.actors();

        // Then the result is empty.
        assert!(actors.is_empty());
    }
}

jinn_slices::crossing_schema!(ServiceStatusUpdate, "ServiceStatusUpdate", trouper::schema::SchemaKind::Event,
    description: "A feature's projection onto its dashboard row (optional lifecycle, description, status message).",
    fields: ["name" => trouper::schema::FieldTy::Str]);

/// Drains the dashboard slice's forward-bridge routes: one relay actor
/// per crossing message, registered on the kameo bus to republish onto
/// the dashboard's trouper topics.
///
/// The relays register synchronously (subscribe is the readiness
/// point), so this may run before or after [`activate`].
pub async fn drain_forward_routes(services: &crate::Services) {
    use crate::common::actor::protocol::event::{
        ActorShutdownCompleted, ActorStarted, ActorStarting,
    };
    use crate::common::trouper_bridge::spawn_one;
    spawn_one::<ActorStarting>(services, &route_entry::<ActorStarting>()).await;
    spawn_one::<ActorStarted>(services, &route_entry::<ActorStarted>()).await;
    spawn_one::<ActorShutdownCompleted>(services, &route_entry::<ActorShutdownCompleted>()).await;
    spawn_one::<ServiceStatusUpdate>(services, &route_entry::<ServiceStatusUpdate>()).await;
    spawn_one::<nav::DashboardNav>(services, &route_entry::<nav::DashboardNav>()).await;
}

/// The staged route entry for a dashboard crossing message.
fn route_entry<M: trouper::schema::Schema>() -> jinn_slices::host::RouteEntry {
    jinn_slices::host::RouteEntry {
        schema_id: M::schema_id(),
        name: "dashboard",
        topic: topic_for::<M>(),
        direction: jinn_slices::host::Direction::Forward,
    }
}

/// The trouper topic a dashboard crossing message publishes onto.
fn topic_for<M: trouper::schema::Schema>() -> trouper::types::Topic {
    use crate::common::trouper_bridge::topics;
    use trouper::types::Topic;
    match M::schema_id().to_string().as_str() {
        id if id.starts_with("DashboardNav") => Topic::new(topics::DASHBOARD),
        id if id.starts_with("SubmitQuakeBarCommand") => Topic::new(topics::QUAKE_BAR),
        _ => Topic::new(topics::FABRIC),
    }
}
