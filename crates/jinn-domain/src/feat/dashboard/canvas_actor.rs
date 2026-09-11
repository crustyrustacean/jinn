//! The dashboard actor — owns the dashboard slice cell on trouper.
//!
//! Aggregates three data sources into a single dashboard view:
//!
//! - **Generic actor lifecycle** — receives the lifecycle events
//!   [`ActorStarting`], [`ActorStarted`], and [`ActorShutdownCompleted`] to
//!   track every actor's `Starting`/`Running`/`Dead` phase.
//! - **Generic service status** — receives [`ServiceStatusUpdate`] events
//!   published by whichever feature owns a service, applying the optional
//!   lifecycle, description, and status message to the named row.
//! - **Keyboard navigation** — receives [`DashboardNav`], bridged onto the
//!   `jinn.dashboard` topic from the dashboard feature's keybind rows.
//!
//! This actor is a feature-agnostic sink: features translate their own
//! state into the generic events, so no feature-specific type appears
//! here. It owns the dashboard's slice cell exclusively: the cell is
//! minted by [`Slices::register`](crate::common::slices::Slices::register)
//! at actor wiring, and this actor holds the one write handle. The
//! renderer and the intent router resolve read handles. Status sources
//! are symmetric producers: they publish events, and this actor is the
//! single sink.
//!
//! The actor runs on the trouper runtime ([`ServiceActor`] tier: a
//! stateless fold into shared state, no journaling). The kameo→canvas
//! bridge ([`crate::common::trouper_bridge`]) translates the bus messages
//! onto its topics; the cell handle cannot ride the runtime's JSON start
//! args, so it is injected through the builder's
//! [`start_with`](trouper::builder::ServiceBuilder::start_with)
//! override.

use trouper::actor::{MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;
use trouper::types::ActorPath;

use crate::common::actor::protocol::event::{ActorShutdownCompleted, ActorStarted, ActorStarting};
use crate::common::trouper_bridge;
use crate::feat::dashboard::nav::DashboardNav;
use crate::feat::dashboard::{ActorLifecycle, DashboardState, ServiceStatusUpdate};
use jinn_slices::TypedCell;

/// The dashboard actor on the canvas runtime.
///
/// Receives lifecycle events, [`ServiceStatusUpdate`], and
/// [`DashboardNav`] on its topics, folding all of them into the slice
/// cell.
pub struct DashboardCanvasActor {
    /// The dashboard's slice cell — minted at wiring, owned here.
    cell: TypedCell<DashboardState>,
}

impl ServiceActor for DashboardCanvasActor {
    async fn start(
        _args: &serde_json::Value,
    ) -> Result<Self, trouper::error_stack::Report<RegistryError>> {
        // Never called: the spawn helper injects the cell via `start_with`.
        Err(
            trouper::error_stack::IntoReport::into_report(RegistryError::InvalidSpec).attach(
                "DashboardCanvasActor is spawned via start_with; start requires the typed cell",
            ),
        )
    }
}

impl DashboardCanvasActor {
    /// Spawns the actor at `dashboard` and subscribes it to both its
    /// topics (`jinn.fabric` + `jinn.dashboard`).
    ///
    /// A successful [`ActorSystem::subscribe`] is the ordering guarantee:
    /// the topic cursors are registered, so every later publish reaches
    /// the actor's inbox. This is what lets the activation sequence be
    /// spawn-then-activate-the-world without missed lifecycle events.
    ///
    /// # Panics
    ///
    /// Panics if the topic subscriptions fail, which can only happen on a
    /// broken actor system; the spawn-then-activate ordering relies on it.
    pub fn spawn(
        system: &std::sync::Arc<ActorSystem>,
        cell: &TypedCell<DashboardState>,
    ) -> ActorPath {
        let path = trouper::builder::spawn_service_builder::<Self>(system)
            .at(ActorPath::new("dashboard"))
            .start_with({
                let cell = cell.clone();
                move || Box::pin(async move { Ok(Self { cell }) })
            })
            .handles::<ActorStarting>()
            .handles::<ActorStarted>()
            .handles::<ActorShutdownCompleted>()
            .handles::<ServiceStatusUpdate>()
            .handles::<DashboardNav>()
            .start();
        #[expect(
            clippy::expect_used,
            reason = "subscription failure is a broken actor system, not a caller bug;                       the spawn-then-activate ordering relies on the cursor being registered"
        )]
        system
            .subscribe(&path, &trouper_bridge::fabric_topic(), None)
            .expect("dashboard actor subscribes to the fabric topic");
        #[expect(
            clippy::expect_used,
            reason = "subscription failure is a broken actor system, not a caller bug"
        )]
        system
            .subscribe(&path, &trouper_bridge::dashboard_topic(), None)
            .expect("dashboard actor subscribes to the dashboard topic");
        path
    }

    /// Folds an [`ActorStarting`] into the cell.
    fn apply_starting(&self, msg: &ActorStarting) {
        self.cell
            .update(|s| s.mark_starting(&msg.name, msg.description.clone()));
    }

    /// Folds an [`ActorStarted`] into the cell.
    fn apply_started(&self, msg: &ActorStarted) {
        self.cell
            .update(|s| s.mark_running(&msg.name, msg.description.clone()));
    }

    /// Folds an [`ActorShutdownCompleted`] into the cell.
    fn apply_shutdown(&self, msg: &ActorShutdownCompleted) {
        self.cell.update(|s| s.mark_dead(&msg.name, None));
    }

    /// Folds a [`ServiceStatusUpdate`] into the cell: the owning
    /// feature's projection onto its row (optional lifecycle, optional
    /// description, optional status message).
    fn apply_service_status(&self, msg: &ServiceStatusUpdate) {
        self.cell.update(|s| apply_service_update(s, msg));
    }

    /// Folds a [`DashboardNav`] into the cell.
    fn apply_nav(&self, msg: DashboardNav) {
        self.cell.update(|s| match msg {
            DashboardNav::Up => s.select_prev(),
            DashboardNav::Down => s.select_next(),
            DashboardNav::First => s.select_first(),
            DashboardNav::Last => s.select_last(),
        });
    }
}

impl MsgHandler<ActorStarting> for DashboardCanvasActor {
    async fn handle(&mut self, msg: ActorStarting, _ctx: &mut MsgCtx<'_>) {
        self.apply_starting(&msg);
    }
}

impl MsgHandler<ActorStarted> for DashboardCanvasActor {
    async fn handle(&mut self, msg: ActorStarted, _ctx: &mut MsgCtx<'_>) {
        self.apply_started(&msg);
    }
}

impl MsgHandler<ActorShutdownCompleted> for DashboardCanvasActor {
    async fn handle(&mut self, msg: ActorShutdownCompleted, _ctx: &mut MsgCtx<'_>) {
        self.apply_shutdown(&msg);
    }
}

impl MsgHandler<ServiceStatusUpdate> for DashboardCanvasActor {
    async fn handle(&mut self, msg: ServiceStatusUpdate, _ctx: &mut MsgCtx<'_>) {
        self.apply_service_status(&msg);
    }
}

impl MsgHandler<DashboardNav> for DashboardCanvasActor {
    async fn handle(&mut self, msg: DashboardNav, _ctx: &mut MsgCtx<'_>) {
        self.apply_nav(msg);
    }
}

/// Apply a generic service status update to the dashboard state.
///
/// The dashboard is a feature-agnostic sink: the owning feature
/// translates its own state and publishes this projection; the fold
/// applies whichever optional fields the event carries (`None`
/// lifecycle leaves the row's phase untouched; `None` description
/// preserves the existing one).
fn apply_service_update(dashboard: &mut DashboardState, update: &ServiceStatusUpdate) {
    if let Some(lifecycle) = update.lifecycle {
        match lifecycle {
            ActorLifecycle::Starting => {
                dashboard.mark_starting(&update.name, update.description.clone());
            }
            ActorLifecycle::Running => {
                dashboard.mark_running(&update.name, update.description.clone());
            }
            ActorLifecycle::Dead => {
                dashboard.mark_dead(&update.name, update.description.clone());
            }
        }
    }
    if update.status_message.is_some() {
        dashboard.set_status_message(&update.name, update.status_message.clone());
    }
}

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
    use crate::common::slices::Slices;
    use crate::feat::dashboard::ActorLifecycle;
    use crate::feat::dashboard::dashboard_slot;

    /// Polls `check` until it passes or the bounded retry budget runs out.
    async fn wait_for(check: impl Fn() -> bool) {
        for _ in 0..200 {
            if check() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("condition never held within the retry budget");
    }

    fn dashboard_entry(
        cell: &TypedCell<DashboardState>,
        name: &str,
    ) -> Option<(ActorLifecycle, Option<String>, Option<String>)> {
        let s = cell.read();
        s.actors()
            .iter()
            .find(|e| e.name == name)
            .map(|e| (e.lifecycle, e.status_message.clone(), e.description.clone()))
    }

    /// Wires one slice cell + dashboard canvas actor into an existing
    /// services container (bridge assumed spawned by the caller).
    fn wire_actor(services: &crate::Services) -> TypedCell<DashboardState> {
        let slices = Slices::new();
        let cell = slices
            .register(dashboard_slot(), DashboardState::new())
            .expect("fresh registry");
        DashboardCanvasActor::spawn(&services.trouper_system, &cell);
        cell
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn actor_starting_event_creates_entry_with_starting_lifecycle() {
        // Given a dashboard canvas actor wired behind the bridge.
        let services = crate::Services::new_fake().await;
        trouper_bridge::spawn_kameo_to_trouper(&services).await;
        let cell = wire_actor(&services);

        // When publishing ActorStarting on the kameo bus.
        services
            .bus
            .publish(ActorStarting {
                name: "llm".to_owned(),
                description: None,
            })
            .await;

        // Then the dashboard shows the actor as Starting.
        wait_for(|| {
            dashboard_entry(&cell, "llm").is_some_and(|(l, _, _)| l == ActorLifecycle::Starting)
        })
        .await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn actor_started_event_transitions_to_running() {
        // Given a dashboard canvas actor wired behind the bridge.
        let services = crate::Services::new_fake().await;
        trouper_bridge::spawn_kameo_to_trouper(&services).await;
        let slices = Slices::new();
        let cell = slices
            .register(dashboard_slot(), DashboardState::new())
            .expect("fresh registry");
        DashboardCanvasActor::spawn(&services.trouper_system, &cell);

        // When publishing ActorStarted on the kameo bus.
        services
            .bus
            .publish(ActorStarted {
                name: "llm".to_owned(),
                description: None,
            })
            .await;

        // Then the dashboard shows the actor as Running.
        wait_for(|| {
            dashboard_entry(&cell, "llm").is_some_and(|(l, _, _)| l == ActorLifecycle::Running)
        })
        .await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn actor_shutdown_event_transitions_to_dead() {
        // Given a dashboard canvas actor whose llm entry is already Running.
        let services = crate::Services::new_fake().await;
        trouper_bridge::spawn_kameo_to_trouper(&services).await;
        let slices = Slices::new();
        let cell = slices
            .register(dashboard_slot(), DashboardState::new())
            .expect("fresh registry");
        DashboardCanvasActor::spawn(&services.trouper_system, &cell);
        services
            .bus
            .publish(ActorStarted {
                name: "llm".to_owned(),
                description: None,
            })
            .await;
        wait_for(|| {
            dashboard_entry(&cell, "llm").is_some_and(|(l, _, _)| l == ActorLifecycle::Running)
        })
        .await;

        // When publishing ActorShutdownCompleted on the kameo bus.
        services
            .bus
            .publish(ActorShutdownCompleted {
                name: "llm".to_owned(),
            })
            .await;

        // Then the dashboard shows the actor as Dead.
        wait_for(|| {
            dashboard_entry(&cell, "llm").is_some_and(|(l, _, _)| l == ActorLifecycle::Dead)
        })
        .await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn service_status_update_marks_row_running_with_description_and_message() {
        // Given a dashboard canvas actor wired behind the bridge.
        let services = crate::Services::new_fake().await;
        trouper_bridge::spawn_kameo_to_trouper(&services).await;
        let slices = Slices::new();
        let cell = slices
            .register(dashboard_slot(), DashboardState::new())
            .expect("fresh registry");
        DashboardCanvasActor::spawn(&services.trouper_system, &cell);

        // When publishing a ServiceStatusUpdate carrying lifecycle,
        // description, and message for a row that does not exist yet.
        services
            .bus
            .publish(ServiceStatusUpdate {
                name: "discord".to_owned(),
                description: Some("Discord gateway bot [Task]".to_owned()),
                lifecycle: Some(ActorLifecycle::Running),
                status_message: Some("Connected".to_owned()),
            })
            .await;

        // Then the row exists as Running with the description and message.
        wait_for(|| {
            dashboard_entry(&cell, "discord").is_some_and(|(l, m, d)| {
                l == ActorLifecycle::Running
                    && m.as_deref() == Some("Connected")
                    && d.as_deref() == Some("Discord gateway bot [Task]")
            })
        })
        .await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn service_status_update_without_lifecycle_leaves_lifecycle_untouched() {
        // Given a dashboard canvas actor whose llm row is already Running.
        let services = crate::Services::new_fake().await;
        trouper_bridge::spawn_kameo_to_trouper(&services).await;
        let slices = Slices::new();
        let cell = slices
            .register(dashboard_slot(), DashboardState::new())
            .expect("fresh registry");
        DashboardCanvasActor::spawn(&services.trouper_system, &cell);
        services
            .bus
            .publish(ActorStarted {
                name: "llm".to_owned(),
                description: None,
            })
            .await;
        wait_for(|| {
            dashboard_entry(&cell, "llm").is_some_and(|(l, _, _)| l == ActorLifecycle::Running)
        })
        .await;

        // When publishing a message-only ServiceStatusUpdate (None lifecycle).
        services
            .bus
            .publish(ServiceStatusUpdate {
                name: "llm".to_owned(),
                description: None,
                lifecycle: None,
                status_message: Some("resolving…".to_owned()),
            })
            .await;

        // Then the message lands but the lifecycle stays Running.
        wait_for(|| {
            dashboard_entry(&cell, "llm").is_some_and(|(l, m, _)| {
                l == ActorLifecycle::Running && m.as_deref() == Some("resolving…")
            })
        })
        .await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn service_status_update_without_message_preserves_existing_message() {
        // Given a dashboard canvas actor whose web-fetch row carries a message.
        let services = crate::Services::new_fake().await;
        trouper_bridge::spawn_kameo_to_trouper(&services).await;
        let slices = Slices::new();
        let cell = slices
            .register(dashboard_slot(), DashboardState::new())
            .expect("fresh registry");
        DashboardCanvasActor::spawn(&services.trouper_system, &cell);
        services
            .bus
            .publish(ServiceStatusUpdate {
                name: "web-fetch".to_owned(),
                description: None,
                lifecycle: None,
                status_message: Some("Chrome 138".to_owned()),
            })
            .await;
        wait_for(|| {
            dashboard_entry(&cell, "web-fetch")
                .is_some_and(|(_, m, _)| m.as_deref() == Some("Chrome 138"))
        })
        .await;

        // When publishing a lifecycle-only ServiceStatusUpdate (None message).
        services
            .bus
            .publish(ServiceStatusUpdate {
                name: "web-fetch".to_owned(),
                description: None,
                lifecycle: Some(ActorLifecycle::Dead),
                status_message: None,
            })
            .await;

        // Then the lifecycle lands but the message is preserved.
        wait_for(|| {
            dashboard_entry(&cell, "web-fetch").is_some_and(|(l, m, _)| {
                l == ActorLifecycle::Dead && m.as_deref() == Some("Chrome 138")
            })
        })
        .await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn dashboard_nav_command_moves_selection() {
        // Given a dashboard canvas actor with three actor rows.
        let services = crate::Services::new_fake().await;
        trouper_bridge::spawn_kameo_to_trouper(&services).await;
        let slices = Slices::new();
        let cell = slices
            .register(dashboard_slot(), DashboardState::new())
            .expect("fresh registry");
        DashboardCanvasActor::spawn(&services.trouper_system, &cell);
        for name in ["a", "b", "c"] {
            services
                .bus
                .publish(ActorStarted {
                    name: name.to_owned(),
                    description: None,
                })
                .await;
        }
        wait_for(|| dashboard_entry(&cell, "c").is_some()).await;

        // When a DashboardNav::Down message arrives.
        services.bus.publish(DashboardNav::Down).await;

        // Then the selection moved to index 1.
        wait_for(|| cell.read().selected_index() == 1).await;
    }

    /// Publishes an `ActorStarted` only AFTER the actor wiring has fully
    /// returned, proving the topic cursor was registered during wiring —
    /// the no-missed-lifecycle-events property the activation sequence
    /// depends on.
    #[rstest::rstest]
    #[tokio::test]
    async fn events_published_after_wiring_are_not_missed() {
        // Given a canvas system with the bridge already spawned.
        let services = crate::Services::new_fake().await;
        trouper_bridge::spawn_kameo_to_trouper(&services).await;

        // When the dashboard activates (spawn + subscribe) and only then
        // an ActorStarted is published.
        let slices = Slices::new();
        let cell = slices
            .register(dashboard_slot(), DashboardState::new())
            .expect("fresh registry");
        DashboardCanvasActor::spawn(&services.trouper_system, &cell);
        services
            .bus
            .publish(ActorStarted {
                name: "late".to_owned(),
                description: None,
            })
            .await;

        // Then the entry still lands — nothing was missed.
        wait_for(|| dashboard_entry(&cell, "late").is_some()).await;
    }
}
