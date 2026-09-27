//! The dashboard actor — owns the dashboard slice cell on trouper.
//!
//! Aggregates three data sources into a single dashboard view:
//!
//! - **The runtime's actor census** — receives [`trouper::ActorLifecycle`],
//!   which the runtime broadcasts system-level at every actor spawn and
//!   stop. This is what makes the dashboard a census rather than a
//!   curated list: a row appears for an actor that publishes no
//!   jinn-level event of any kind, and no spawn site in any slice needs
//!   to know the dashboard exists.
//! - **Generic service status** — receives [`ServiceStatusUpdate`]
//!   published by whichever feature owns a service, adding a
//!   human-written description and status message to that row.
//! - **Keyboard navigation** — receives [`DashboardNav`], bridged from
//!   the dashboard feature's keybind rows.
//!
//! The two sources are deliberately asymmetric: the runtime decides
//! *whether a row exists and whether it is alive*, a feature decides
//! *what its row says*. A feature can never bring an actor into being
//! by announcing it, and can never contradict the runtime's verdict on
//! whether it is alive.
//!
//! This actor is a feature-agnostic sink: features translate their own
//! state into the generic event, so no feature-specific type appears
//! here. It owns the dashboard's slice cell exclusively: the cell is
//! minted by [`Slices::register`](jinn_slices::Slices::register)
//! at actor wiring, and this actor holds the one write handle. The
//! renderer and the intent router resolve read handles.
//!
//! The actor runs on the trouper runtime ([`ServiceActor`] tier: a
//! stateless fold into shared state, no journaling). The cell handle
//! cannot ride the runtime's JSON start args, so it is injected
//! through the builder's
//! [`start_with`](trouper::builder::ServiceBuilder::start_with)
//! override.

use trouper::actor::ActorPath;
use trouper::actor::{MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;

use crate::nav::DashboardNav;
use crate::{ActorLifecycle, DashboardState, ServiceStatusUpdate};
use jinn_slices::TypedCell;
use trouper::LifecycleState;

/// The dashboard actor on the canvas runtime.
///
/// Receives [`trouper::ActorLifecycle`], [`ServiceStatusUpdate`], and
/// [`DashboardNav`], folding all of them into the slice cell.
pub struct DashboardCanvasActor {
    /// The dashboard's slice cell — minted at wiring, owned here.
    cell: TypedCell<DashboardState>,
}

impl ServiceActor for DashboardCanvasActor {
    async fn start(
        _args: &trouper::json::Json,
    ) -> Result<Self, error_stack::Report<RegistryError>> {
        // Never called: the spawn helper injects the cell via `start_with`.
        Err(
            error_stack::IntoReport::into_report(RegistryError::InvalidSpec).attach(
                "DashboardCanvasActor is spawned via start_with; start requires the typed cell",
            ),
        )
    }
}

impl DashboardCanvasActor {
    /// Spawns the actor at `dashboard` and subscribes it to the census
    /// and feature-status schemas.
    ///
    /// A successful spawn is the ordering guarantee: `.handles` registers
    /// the schemas, so every later broadcast reaches this actor's inbox.
    /// This is what lets the activation sequence be spawn-then-activate-
    /// the-world without missing a spawn announcement.
    ///
    /// # Panics
    ///
    /// Panics if the spawn fails, which can only happen on a broken actor
    /// system; the spawn-then-activate ordering relies on it.
    pub fn spawn(system: &ActorSystem, cell: &TypedCell<DashboardState>) -> ActorPath {
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(ActorPath::new("dashboard"))
            // Deep inbox: the startup burst (one announcement per actor,
            // plus a per-row status message where a feature sends one)
            // must not fill the dashboard's inbox — a full inbox stalls
            // dispatch while the retained log evicts, silently dropping
            // rows.
            .mailbox(64 * 1024, trouper::inbox::OverloadPolicy::Block)
            .start_with({
                let cell = cell.clone();
                move || Box::pin(async move { Ok(Self { cell }) })
            })
            .handles::<trouper::ActorLifecycle>()
            .handles::<ServiceStatusUpdate>()
            .handles::<DashboardNav>()
            .start()
    }

    /// Folds a runtime lifecycle announcement into the cell.
    fn apply_lifecycle(&self, msg: &trouper::ActorLifecycle) {
        self.cell.update(|s| {
            let name = msg.path.to_string();
            match msg.state {
                LifecycleState::Running => s.mark_running(name, None),
                // Every stop state lands on the same `Dead` row, but
                // each carries its own reason so the view can tell a
                // passivation from a crash. `Passivated` in particular is
                // NOT a failure: the actor re-spawns on the next send.
                state => s.mark_stopped(name, stop_reason_text(state)),
            }
        });
    }

    /// Folds a [`ServiceStatusUpdate`] into the cell: the owning
    /// feature's projection onto its row (optional lifecycle, optional
    /// description, optional status message).
    fn apply_service_status(&self, msg: &ServiceStatusUpdate) {
        self.cell.update(|s| apply_service_update(s, msg));
    }

    /// Folds a [`DashboardNav`] into the cell.
    fn apply_nav(&self, msg: &DashboardNav) {
        self.cell.update(|s| match msg {
            DashboardNav::Up => s.select_prev(),
            DashboardNav::Down => s.select_next(),
            DashboardNav::First => s.select_first(),
            DashboardNav::Last => s.select_last(),
        });
    }
}

/// The Notes-column phrase for a stopped actor's runtime state.
///
/// Every stop state gets a distinct, human-readable phrase, and none
/// reuses the word "Dead" — the State column already says that, and
/// repeating it here would waste the column that exists to say *why*.
fn stop_reason_text(state: LifecycleState) -> String {
    match state {
        LifecycleState::Running => "running".to_owned(),
        LifecycleState::Normal => "stopped normally".to_owned(),
        LifecycleState::Crashed => "crashed (supervisor declined restart)".to_owned(),
        LifecycleState::Escalated => "escalated (restart budget exhausted)".to_owned(),
        LifecycleState::Passivated => "passivated (idle; re-spawns on next send)".to_owned(),
        LifecycleState::Shutdown => "stopped by shutdown".to_owned(),
    }
}

impl MsgHandler<trouper::ActorLifecycle> for DashboardCanvasActor {
    async fn handle(&mut self, msg: &trouper::ActorLifecycle, _ctx: &mut MsgCtx<'_>) {
        self.apply_lifecycle(msg);
    }
}

impl MsgHandler<ServiceStatusUpdate> for DashboardCanvasActor {
    async fn handle(&mut self, msg: &ServiceStatusUpdate, _ctx: &mut MsgCtx<'_>) {
        self.apply_service_status(msg);
    }
}

impl MsgHandler<DashboardNav> for DashboardCanvasActor {
    async fn handle(&mut self, msg: &DashboardNav, _ctx: &mut MsgCtx<'_>) {
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
    // The description applies INDEPENDENTLY of the lifecycle. It used to
    // be a by-product of the lifecycle arm, so a `None` lifecycle
    // silently dropped the description — and both real publishers
    // (discord) legitimately send `None` lifecycle on some transitions
    // while still meaning to describe the row. Losing a description
    // because a publisher omitted an unrelated optional field is a
    // coupling that has no reason to exist.
    if update.description.is_some() {
        dashboard.set_description(&update.name, update.description.clone());
    }
    if let Some(lifecycle) = update.lifecycle {
        match lifecycle {
            ActorLifecycle::Starting => {
                dashboard.mark_starting(&update.name, None);
            }
            ActorLifecycle::Running => {
                dashboard.mark_running(&update.name, None);
            }
            ActorLifecycle::Dead => {
                dashboard.mark_dead(&update.name, None);
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
    use crate::contracts::ServiceStatusUpdate;
    use crate::dashboard_slot;
    use crate::nav::DashboardNav;
    use crate::state::DashboardState;
    use jinn_slices::Slices;
    use jinn_slices::TypedCell;
    use jinn_testutil::TestFabric;
    use trouper::actor::ActorKind;
    use trouper::schema::Schema;

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

    /// A snapshot of one dashboard row, by field name.
    ///
    /// Named fields rather than a positional tuple: these tests assert
    /// on one field at a time, and `row.status` says which one a failure
    /// is about.
    #[derive(Debug, Clone)]
    struct Row {
        lifecycle: ActorLifecycle,
        status: Option<String>,
        description: Option<String>,
        reason: Option<String>,
    }

    /// Reads the named row out of the dashboard cell.
    fn row(cell: &TypedCell<DashboardState>, name: &str) -> Option<Row> {
        let s = cell.read();
        s.actors().iter().find(|e| e.name == name).map(|e| Row {
            lifecycle: e.lifecycle,
            status: e.status_message.clone(),
            description: e.description.clone(),
            reason: e.stop_reason.clone(),
        })
    }

    /// Wires one dashboard cell + canvas actor onto the test fabric.
    fn wire_actor(fabric: &TestFabric) -> TypedCell<DashboardState> {
        let slices = Slices::new();
        let cell = slices
            .register(dashboard_slot(), DashboardState::new())
            .expect("fresh registry");
        DashboardCanvasActor::spawn(fabric.system(), &cell);
        cell
    }

    /// A bare service actor that handles and emits nothing, spawned at
    /// `name`. Spawning one of these is the strongest form of the
    /// census claim: the runtime announces it and nothing in jinn ever
    /// names it, so a row can only come from the runtime.
    struct CensusProbe;

    impl trouper::actor::ServiceActor for CensusProbe {
        fn manifest() -> trouper::schema::ActorManifest {
            trouper::schema::ActorManifest::new()
        }
        async fn start(
            _args: &trouper::json::Json,
        ) -> Result<Self, error_stack::Report<trouper::registry::RegistryError>> {
            Ok(Self)
        }
    }

    /// Spawns a [`CensusProbe`] and returns its path.
    fn spawn_probe(system: &ActorSystem, name: &str) -> ActorPath {
        trouper::builder::spawn_service_builder::<CensusProbe>(system)
            .at(ActorPath::new(name))
            .start()
    }

    /// A runtime spawn announcement for `path`.
    fn running(path: &str) -> trouper::ActorLifecycle {
        trouper::ActorLifecycle {
            path: ActorPath::new(path),
            state: LifecycleState::Running,
            kind: ActorKind::Service,
        }
    }

    /// A runtime stop announcement for `path` with `state`.
    fn stopped(path: &str, state: LifecycleState) -> trouper::ActorLifecycle {
        trouper::ActorLifecycle {
            path: ActorPath::new(path),
            state,
            kind: ActorKind::Service,
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn a_spawn_announcement_creates_a_running_row() {
        // Given a dashboard canvas actor subscribed to the census.
        let fabric = TestFabric::new();
        let cell = wire_actor(&fabric);

        // When a spawn announcement lands.
        fabric.send_to_topic(running("session")).await;

        // Then the actor has a row, and it reads Running.
        wait_for(|| row(&cell, "session").is_some_and(|r| r.lifecycle == ActorLifecycle::Running))
            .await;
    }

    /// THE decisive test: a row appears for an actor that publishes no
    /// jinn-level event of any kind. The dashboard used to learn about
    /// actors only from hand-written `ActorStarting` publishes at two
    /// spawn sites, so it listed two of roughly forty-five live actors.
    #[rstest::rstest]
    #[tokio::test]
    async fn an_actor_that_publishes_nothing_gets_a_row() {
        // Given a dashboard canvas actor subscribed to the census.
        let fabric = TestFabric::new();
        let cell = wire_actor(&fabric);

        // When the runtime announces a spawn for an arbitrary path —
        // no `ServiceStatusUpdate`, no feature of any kind involved.
        fabric.send_to_topic(running("mcp/coordinator")).await;

        // Then the row exists purely because the runtime said so.
        wait_for(|| {
            row(&cell, "mcp/coordinator").is_some_and(|r| r.lifecycle == ActorLifecycle::Running)
        })
        .await;
    }

    /// The dashboard's census is fed by the runtime's announcement, not
    /// by any observation handler — so it must work on a system where
    /// one was never installed. `TestFabric` builds a production-config
    /// system, which installs none.
    #[rstest::rstest]
    #[tokio::test]
    async fn rows_appear_without_any_observation_handler() {
        // Given a fabric with no observation handler installed.
        let fabric = TestFabric::new();
        let cell = wire_actor(&fabric);

        // When actors spawn.
        let paths: Vec<String> = ["a", "b", "c"]
            .iter()
            .map(|name| spawn_probe(fabric.system(), name).to_string())
            .collect();

        // Then every row exists, keyed by the runtime's own path.
        wait_for(|| {
            let s = cell.read();
            paths
                .iter()
                .all(|p| s.actors().iter().any(|e| e.name == *p))
        })
        .await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn a_stop_announcement_marks_the_row_dead() {
        // Given a wired actor with a running row.
        let fabric = TestFabric::new();
        let cell = wire_actor(&fabric);
        fabric.send_to_topic(running("llm")).await;
        wait_for(|| row(&cell, "llm").is_some()).await;

        // When a stop announcement arrives.
        fabric
            .send_to_topic(stopped("llm", LifecycleState::Normal))
            .await;

        // Then the row reads Dead.
        wait_for(|| row(&cell, "llm").is_some_and(|r| r.lifecycle == ActorLifecycle::Dead)).await;
    }

    /// `Passivated` is not a failure: the runtime evicted an idle actor
    /// and will re-spawn it on the next send. The Notes column must not
    /// present it as a crash.
    #[rstest::rstest]
    #[tokio::test]
    async fn a_passivated_stop_reads_as_idle_not_crashed() {
        // Given a wired actor with a running row.
        let fabric = TestFabric::new();
        let cell = wire_actor(&fabric);
        fabric.send_to_topic(running("idle-worker")).await;
        wait_for(|| row(&cell, "idle-worker").is_some()).await;

        // When the runtime announces a passivation.
        fabric
            .send_to_topic(stopped("idle-worker", LifecycleState::Passivated))
            .await;

        // Then the row is Dead with a passivation phrase, not a crash one.
        wait_for(|| {
            row(&cell, "idle-worker")
                .and_then(|r| r.reason)
                .is_some_and(|r| r.contains("passivated"))
        })
        .await;
        let reason = row(&cell, "idle-worker").unwrap().reason.unwrap();
        assert!(
            !reason.contains("crashed"),
            "passivation must not read as a crash: {reason}"
        );
    }

    #[rstest::rstest]
    #[case(LifecycleState::Normal, "stopped normally")]
    #[case(LifecycleState::Crashed, "crashed")]
    #[case(LifecycleState::Escalated, "escalated")]
    #[case(LifecycleState::Passivated, "passivated")]
    #[case(LifecycleState::Shutdown, "shutdown")]
    #[tokio::test]
    async fn each_stop_state_renders_its_own_distinct_phrase(
        #[case] state: LifecycleState,
        #[case] expected: &str,
    ) {
        // Given a wired actor with a running row.
        let fabric = TestFabric::new();
        let cell = wire_actor(&fabric);
        fabric.send_to_topic(running("subject")).await;
        wait_for(|| row(&cell, "subject").is_some()).await;

        // When the runtime announces that stop state.
        fabric.send_to_topic(stopped("subject", state)).await;

        // Then the row carries that state's own phrase.
        wait_for(|| {
            row(&cell, "subject")
                .and_then(|r| r.reason)
                .is_some_and(|r| r.contains(expected))
        })
        .await;
    }

    /// One row, one Notes cell. A feature's status message is the more
    /// specific statement, so it wins; the stop reason shows only when
    /// the feature has nothing to say.
    #[rstest::rstest]
    #[tokio::test]
    async fn a_stop_reason_is_stored_alongside_a_feature_status_message() {
        // Given a wired actor whose row carries a feature status message.
        let fabric = TestFabric::new();
        let cell = wire_actor(&fabric);
        fabric.send_to_topic(running("discord")).await;
        wait_for(|| row(&cell, "discord").is_some()).await;
        fabric
            .send_to_topic(ServiceStatusUpdate {
                name: "discord".to_owned(),
                description: Some("Discord gateway".to_owned()),
                lifecycle: None,
                status_message: Some("connected".to_owned()),
            })
            .await;
        wait_for(|| {
            row(&cell, "discord").is_some_and(|r| r.status.as_deref() == Some("connected"))
        })
        .await;

        // When the actor stops.
        fabric
            .send_to_topic(stopped("discord", LifecycleState::Normal))
            .await;

        // Then the row is Dead, the feature message is intact, and the
        // stop reason is held for the view to fall back to.
        wait_for(|| row(&cell, "discord").is_some_and(|r| r.lifecycle == ActorLifecycle::Dead))
            .await;
        let row = row(&cell, "discord").unwrap();
        assert_eq!(row.status.as_deref(), Some("connected"));
        assert_eq!(row.description.as_deref(), Some("Discord gateway"));
        assert!(
            row.reason
                .as_ref()
                .is_some_and(|r| r.contains("stopped normally")),
            "the stop reason is retained: {:?}",
            row.reason
        );
    }

    /// A stop reason describes a state the actor has LEFT. A row that is
    /// live again must not keep claiming it is stopped.
    #[rstest::rstest]
    #[tokio::test]
    async fn a_respawn_clears_the_stale_stop_reason() {
        // Given a wired actor whose row was stopped.
        let fabric = TestFabric::new();
        let cell = wire_actor(&fabric);
        fabric.send_to_topic(running("flaky")).await;
        wait_for(|| row(&cell, "flaky").is_some()).await;
        fabric
            .send_to_topic(stopped("flaky", LifecycleState::Crashed))
            .await;
        wait_for(|| row(&cell, "flaky").is_some_and(|r| r.reason.is_some())).await;

        // When the runtime announces it running again — which is exactly
        // what a supervised restart does, indistinguishably.
        fabric.send_to_topic(running("flaky")).await;

        // Then the stale reason is gone.
        wait_for(|| row(&cell, "flaky").is_some_and(|r| r.lifecycle == ActorLifecycle::Running))
            .await;
        assert_eq!(
            row(&cell, "flaky").unwrap().reason,
            None,
            "a live row must not carry a stop reason"
        );
    }

    /// A stop announcement for a path the dashboard never saw start must
    /// still produce a row, or a missed spawn announcement would silently
    /// drop the actor from the census forever.
    #[rstest::rstest]
    #[tokio::test]
    async fn a_stop_for_an_unseen_actor_still_creates_its_row() {
        // Given a wired actor with no row for this path.
        let fabric = TestFabric::new();
        let cell = wire_actor(&fabric);

        // When a stop announcement arrives for an actor it never saw.
        fabric
            .send_to_topic(stopped("vanished", LifecycleState::Shutdown))
            .await;

        // Then the row exists and reads Dead.
        wait_for(|| row(&cell, "vanished").is_some_and(|r| r.lifecycle == ActorLifecycle::Dead))
            .await;
    }

    /// The dashboard folds the runtime's OWN type. Delivery is by schema
    /// id, so a schema-identical mirror would silently drop every
    /// announcement — the exact failure this slice once had. Pinning the
    /// type identity makes a future mirror a compile error rather than
    /// an empty dashboard.
    #[rstest::rstest]
    #[test]
    fn the_census_message_is_the_runtime_type_under_its_bare_name() {
        // Given the type the dashboard declares `.handles` on.
        let announcement = running("llm");

        // When reading its schema id and round-tripping it.
        let id = trouper::ActorLifecycle::schema_id().to_string();
        let roundtripped: trouper::ActorLifecycle =
            serde_json::from_value(serde_json::to_value(&announcement).unwrap()).unwrap();

        // Then the id is the bare type name (no version component), and
        // the payload survived the trip.
        assert_eq!(id, "ActorLifecycle", "schema id was {id}");
        assert_eq!(roundtripped.path, ActorPath::new("llm"));
        assert_eq!(roundtripped.state, LifecycleState::Running);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn a_feature_status_message_sets_the_notes_column() {
        // Given a wired actor with a running row.
        let fabric = TestFabric::new();
        let cell = wire_actor(&fabric);
        fabric.send_to_topic(running("sample-actor")).await;
        wait_for(|| row(&cell, "sample-actor").is_some()).await;

        // When a ServiceStatusUpdate projection arrives with a status message.
        fabric
            .send_to_topic(ServiceStatusUpdate {
                name: "sample-actor".to_owned(),
                description: None,
                lifecycle: None,
                status_message: Some("3 urls verified".to_owned()),
            })
            .await;

        // Then the row carries the message.
        wait_for(|| {
            row(&cell, "sample-actor")
                .is_some_and(|r| r.status.as_deref() == Some("3 urls verified"))
        })
        .await;
    }

    /// A description must not depend on a lifecycle opinion. The
    /// discord publisher sends `None` lifecycle on its `Disconnected`
    /// transition while still supplying a description, and the fold used
    /// to apply the description only inside the lifecycle arm — so that
    /// row silently lost its description.
    #[rstest::rstest]
    #[tokio::test]
    async fn a_description_applies_even_without_a_lifecycle_opinion() {
        // Given a wired actor with a running row.
        let fabric = TestFabric::new();
        let cell = wire_actor(&fabric);
        fabric.send_to_topic(running("gateway")).await;
        wait_for(|| row(&cell, "gateway").is_some()).await;

        // When a status update carries a description but no lifecycle.
        fabric
            .send_to_topic(ServiceStatusUpdate {
                name: "gateway".to_owned(),
                description: Some("Discord gateway".to_owned()),
                lifecycle: None,
                status_message: Some("disconnected".to_owned()),
            })
            .await;

        // Then the description lands.
        wait_for(|| {
            row(&cell, "gateway")
                .and_then(|r| r.description)
                .is_some_and(|d| d == "Discord gateway")
        })
        .await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn nav_messages_move_the_selection_cursor() {
        // Given a wired actor with three named entries among its rows.
        // NOTE: the census also lists the dashboard actor itself, so
        // this asserts on the named rows, never on a total count.
        let fabric = TestFabric::new();
        let cell = wire_actor(&fabric);
        for name in ["a", "b", "c"] {
            fabric.send_to_topic(running(name)).await;
        }
        wait_for(|| {
            let s = cell.read();
            ["a", "b", "c"]
                .iter()
                .all(|n| s.actors().iter().any(|e| e.name == *n))
        })
        .await;

        // When DashboardNav::Down envelopes arrive twice.
        fabric.send_to_topic(DashboardNav::Down).await;
        fabric.send_to_topic(DashboardNav::Down).await;

        // Then the cursor advances twice.
        wait_for(|| cell.read().selected_index() == 2).await;
    }
}
