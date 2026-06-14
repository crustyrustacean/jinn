//! Persona scan actor - scans and loads persona files on command.
//!
//! Subscribes to [`RescanPersonas`] commands, scans the personas directory,
//! and publishes [`PersonasLoaded`] events with the results.

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::feat::context::protocol::command::RescanPersonas;
use crate::feat::context::protocol::event::PersonasLoaded;
use crate::feat::persona::scan_personas_merged;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::feat::session_lifecycle::protocol::event::{
    SessionCreated, SessionCwdChanged, SessionSetupCompleted,
};
use crate::init::env_init_actor::EnvironmentLoaded;
use kameo::prelude::{Actor, ActorRef, Context, Message};

/// Scans and loads persona files on `RescanPersonas`.
///
/// On command, scans user and system persona directories, parses all
/// `*.md` files, and publishes `PersonasLoaded` with the results.
pub struct PersonaScanActor {
    deps: ActorDeps,
}

/// Dependencies for spawning a [`PersonaScanActor`].
#[derive(Clone)]
pub struct PersonaScanActorDeps {
    /// Universal actor dependencies (bus, services, etc.).
    pub deps: ActorDeps,
}

impl Actor for PersonaScanActor {
    type Args = PersonaScanActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.clone().recipient::<RescanPersonas>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<EnvironmentLoaded>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionCreated>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionSetupCompleted>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionLoadCompleted>())
            .await;
        args.deps
            .subscribe(actor_ref.recipient::<SessionCwdChanged>())
            .await;

        Ok(Self { deps: args.deps })
    }
}

impl Message<RescanPersonas> for PersonaScanActor {
    type Reply = ();

    async fn handle(
        &mut self,
        _msg: RescanPersonas,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.run_scan().await;
    }
}

impl Message<EnvironmentLoaded> for PersonaScanActor {
    type Reply = ();

    async fn handle(&mut self, _msg: EnvironmentLoaded, _ctx: &mut Context<Self, Self::Reply>) {
        self.run_scan().await;
    }
}

impl Message<SessionCreated> for PersonaScanActor {
    type Reply = ();

    async fn handle(&mut self, _msg: SessionCreated, _ctx: &mut Context<Self, Self::Reply>) {
        self.run_scan().await;
    }
}

impl Message<SessionSetupCompleted> for PersonaScanActor {
    type Reply = ();

    async fn handle(&mut self, _msg: SessionSetupCompleted, _ctx: &mut Context<Self, Self::Reply>) {
        self.run_scan().await;
    }
}

impl Message<SessionLoadCompleted> for PersonaScanActor {
    type Reply = ();

    async fn handle(&mut self, _msg: SessionLoadCompleted, _ctx: &mut Context<Self, Self::Reply>) {
        self.run_scan().await;
    }
}

impl Message<SessionCwdChanged> for PersonaScanActor {
    type Reply = ();

    async fn handle(&mut self, _msg: SessionCwdChanged, _ctx: &mut Context<Self, Self::Reply>) {
        self.run_scan().await;
    }
}

impl BusPublish for PersonaScanActor {
    fn bus(&self) -> &crate::common::services::bus_service::BusService {
        self.deps.bus()
    }
}

impl PersonaScanActor {
    /// Runs the blocking scan and publishes the result.
    async fn run_scan(&self) {
        let paths = self.deps.services.paths.clone();
        let result = tokio::task::spawn_blocking(move || {
            scan_personas_merged(&paths.personas_dir(), &paths.system_personas_dir())
        })
        .await;

        match result {
            Ok(personas) => {
                tracing::info!(count = personas.len(), "rescanned personas");
                self.publish(PersonasLoaded {
                    personas,
                    error: None,
                })
                .await;
            }
            Err(join_error) => {
                tracing::error!("persona rescan task panicked: {join_error}");
                self.publish(PersonasLoaded {
                    personas: vec![],
                    error: Some(format!("rescan task failed: {join_error}")),
                })
                .await;
            }
        }
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

    use std::time::Duration;

    use crate::common::app_paths::AppPaths;
    use crate::common::bus::test_harness::{TestHarness, await_recorded};
    use crate::common::services::test_services::TestServices;
    use crate::feat::context::protocol::command::RescanPersonas;
    use crate::feat::context::protocol::event::PersonasLoaded;

    use super::{PersonaScanActor, PersonaScanActorDeps};

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_personas_command_publishes_personas_loaded() {
        // Given an actor with a temp directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let harness = TestHarness::new().await;
        let services = TestServices::builder()
            .paths(AppPaths::new_in(dir.path()))
            .with_bus(harness.bus())
            .build();
        let _actor = harness
            .spawn_actor::<PersonaScanActor>(PersonaScanActorDeps {
                deps: crate::common::actor_deps::ActorDeps { services },
            })
            .await;
        let recorder = harness.spawn_recorder::<PersonasLoaded>().await;

        // When publishing RescanPersonas.
        harness.publish(RescanPersonas).await;

        // Then PersonasLoaded is published.
        let events = await_recorded(&recorder, 1, Duration::from_secs(2)).await;
        assert_eq!(events.len(), 1);
        assert!(events[0].error.is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_personas_empty_dir_publishes_empty_loaded() {
        // Given an actor with an empty temp directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let harness = TestHarness::new().await;
        let services = TestServices::builder()
            .paths(AppPaths::new_in(dir.path()))
            .with_bus(harness.bus())
            .build();
        let _actor = harness
            .spawn_actor::<PersonaScanActor>(PersonaScanActorDeps {
                deps: crate::common::actor_deps::ActorDeps { services },
            })
            .await;
        let recorder = harness.spawn_recorder::<PersonasLoaded>().await;

        // When publishing RescanPersonas.
        harness.publish(RescanPersonas).await;

        // Then PersonasLoaded has empty list.
        let events = await_recorded(&recorder, 1, Duration::from_secs(2)).await;
        assert!(events[0].personas.is_empty());
        assert!(events[0].error.is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_personas_nonexistent_dir_publishes_empty_loaded() {
        // Given an actor with a nonexistent directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let harness = TestHarness::new().await;
        let services = TestServices::builder()
            .paths(AppPaths::new_in(dir.path()))
            .with_bus(harness.bus())
            .build();
        let _actor = harness
            .spawn_actor::<PersonaScanActor>(PersonaScanActorDeps {
                deps: crate::common::actor_deps::ActorDeps { services },
            })
            .await;
        let recorder = harness.spawn_recorder::<PersonasLoaded>().await;

        // When publishing RescanPersonas.
        harness.publish(RescanPersonas).await;

        // Then PersonasLoaded has empty list.
        let events = await_recorded(&recorder, 1, Duration::from_secs(2)).await;
        assert!(events[0].personas.is_empty());
        assert!(events[0].error.is_none());
    }

    #[tokio::test]
    async fn environment_loaded_triggers_scan() {
        // Given a persona scan actor subscribed to EnvironmentLoaded.
        let dir = tempfile::tempdir().expect("create temp dir");
        let harness = TestHarness::new().await;
        let services = TestServices::builder()
            .paths(AppPaths::new_in(dir.path()))
            .with_bus(harness.bus())
            .build();
        let _actor = harness
            .spawn_actor::<PersonaScanActor>(PersonaScanActorDeps {
                deps: crate::common::actor_deps::ActorDeps { services },
            })
            .await;
        let recorder = harness.spawn_recorder::<PersonasLoaded>().await;

        // When publishing EnvironmentLoaded.
        harness
            .publish(crate::init::env_init_actor::EnvironmentLoaded {
                config: crate::ProvidersConfig {
                    providers: vec![],
                    aliases: vec![],
                    default_provider: None,
                    alloys: vec![],
                },
            })
            .await;

        // Then PersonasLoaded is published.
        let events = await_recorded(&recorder, 1, Duration::from_secs(2)).await;
        assert_eq!(events.len(), 1);
    }
}
