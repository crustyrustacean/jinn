//! Persona scan actor — scans and loads persona files on command.
//!
//! Subscribes to [`RescanPersonas`](crate::protocol::Command::RescanPersonas) commands,
//! scans the personas directory, and emits [`PersonasLoaded`] events with the results.

use crate::common::actor::scan_actor::NoDirectMsg;
use crate::common::actor::{Actor, ActorContext, ActorEnvelope};
use crate::common::app_paths::AppPaths;
use crate::feat::context::protocol::command::RescanPersonas;
use crate::feat::context::protocol::event::PersonasLoaded;
use crate::feat::persona::scan_personas_merged;
use crate::protocol::{Command, Event};

/// Dependencies for [`PersonaScanActor`].
pub struct PersonaScanActorDeps {
    /// Application paths for resolving scan directories.
    pub paths: AppPaths,
}

/// Scans and loads persona files on `RescanPersonas`.
///
/// On command, scans user and system persona directories, parses all
/// `*.md` files, and emits `PersonasLoaded` with the results.
pub struct PersonaScanActor {
    /// Application paths for resolving scan directories.
    paths: AppPaths,
}

impl Actor for PersonaScanActor {
    type Message = NoDirectMsg;
    type Deps = PersonaScanActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("Scans and loads persona files from ~/.config/nullslop/personas");
        ctx.subscribe_command::<RescanPersonas>();
        Self { paths: deps.paths }
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, ctx: &ActorContext) {
        if let ActorEnvelope::Command(command) = msg {
            self.handle_command(&command, ctx).await;
        }
    }
}

impl PersonaScanActor {
    /// Dispatches incoming commands.
    async fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        if matches!(command, Command::RescanPersonas(..)) {
            self.run_scan(ctx).await;
        }
    }

    /// Runs the blocking scan and emits the result.
    async fn run_scan(&self, ctx: &ActorContext) {
        let paths = self.paths.clone();
        let result = tokio::task::spawn_blocking(move || {
            scan_personas_merged(&paths.personas_dir(), &paths.system_personas_dir())
        })
        .await;

        match result {
            Ok(personas) => {
                tracing::info!(count = personas.len(), "rescanned personas");
                let _ = ctx.send_event(Event::PersonasLoaded(PersonasLoaded {
                    personas,
                    error: None,
                }));
            }
            Err(join_error) => {
                tracing::error!("persona rescan task panicked: {join_error}");
                let _ = ctx.send_event(Event::PersonasLoaded(PersonasLoaded {
                    personas: vec![],
                    error: Some(format!("rescan task failed: {join_error}")),
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use std::sync::Arc;

    use crate::common::actor::{Actor, ActorContext, ActorEnvelope, MessageSink, RecordingSink};
    use crate::common::app_paths::AppPaths;
    use crate::feat::context::protocol::command::RescanPersonas;
    use crate::feat::context::protocol::event::PersonasLoaded;
    use crate::protocol::{Command, Event};

    use super::*;

    fn find_personas_loaded(events: &[Event]) -> Option<&PersonasLoaded> {
        for evt in events {
            if let Event::PersonasLoaded(payload) = evt {
                return Some(payload);
            }
        }
        None
    }

    fn create_actor(
        dir: &tempfile::TempDir,
    ) -> (PersonaScanActor, Arc<RecordingSink>, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("persona-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let deps = PersonaScanActorDeps {
            paths: AppPaths::new_in(dir.path()),
        };
        let actor = PersonaScanActor::activate(deps, &mut ctx);
        (actor, sink, ctx)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_personas_command_emits_personas_loaded() {
        // Given an actor with a temp directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let (mut actor, sink, ctx) = create_actor(&dir);

        // When processing RescanPersonas command.
        actor
            .handle(
                ActorEnvelope::Command(Command::RescanPersonas(RescanPersonas)),
                &ctx,
            )
            .await;

        // Then PersonasLoaded event is emitted.
        let events = sink.events();
        let loaded = find_personas_loaded(&events);
        assert!(loaded.is_some());
        let loaded = loaded.expect("should have PersonasLoaded");
        assert!(loaded.error.is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_personas_empty_dir_emits_empty_loaded() {
        // Given an actor with an empty temp directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let (mut actor, sink, ctx) = create_actor(&dir);

        // When processing RescanPersonas command.
        actor
            .handle(
                ActorEnvelope::Command(Command::RescanPersonas(RescanPersonas)),
                &ctx,
            )
            .await;

        // Then PersonasLoaded has empty list.
        let events = sink.events();
        let loaded = find_personas_loaded(&events).expect("should have PersonasLoaded");
        assert!(loaded.personas.is_empty());
        assert!(loaded.error.is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_personas_nonexistent_dir_emits_empty_loaded() {
        // Given an actor with a nonexistent directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let (mut actor, sink, ctx) = create_actor(&dir);

        // When processing RescanPersonas command.
        actor
            .handle(
                ActorEnvelope::Command(Command::RescanPersonas(RescanPersonas)),
                &ctx,
            )
            .await;

        // Then PersonasLoaded has empty list.
        let events = sink.events();
        let loaded = find_personas_loaded(&events).expect("should have PersonasLoaded");
        assert!(loaded.personas.is_empty());
        assert!(loaded.error.is_none());
    }
}
