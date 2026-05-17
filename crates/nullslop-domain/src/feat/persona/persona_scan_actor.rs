//! Persona scanning configuration for the generic scan actor.
//!
//! Subscribes to [`RescanPersonas`](crate::protocol::Command::RescanPersonas) commands,
//! scans the personas directory (path injected via [`ActorContext`] data), and emits
//! [`PersonasLoaded`] events with the results.

use crate::common::actor::ActorContext;
use crate::common::actor::scan_actor::{ScanActor, ScanConfig};
use crate::common::app_paths::AppPaths;
use crate::feat::context::protocol::command::RescanPersonas;
use crate::feat::context::protocol::event::PersonasLoaded;
use crate::feat::persona::scan_personas_merged;
use crate::protocol::{Command, Event};

/// Persona scan configuration for [`ScanActor`].
///
/// On `RescanPersonas`, scans the injected directory path, parses all
/// `*.md` files, and emits `PersonasLoaded` with the results.
pub struct PersonaScanConfig;

impl ScanConfig for PersonaScanConfig {
    type Output = Vec<crate::feat::persona::Persona>;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<RescanPersonas>();
        Self
    }

    fn is_rescan_command(command: &Command) -> bool {
        matches!(command, Command::RescanPersonas(..))
    }

    fn scan(paths: &AppPaths) -> Vec<crate::feat::persona::Persona> {
        scan_personas_merged(&paths.personas_dir(), &paths.system_personas_dir())
    }

    fn on_success(
        personas: Vec<crate::feat::persona::Persona>,
        _config: &Self,
        ctx: &ActorContext,
    ) {
        tracing::info!(count = personas.len(), "rescanned personas");
        let _ = ctx.send_event(Event::PersonasLoaded(PersonasLoaded {
            personas,
            error: None,
        }));
    }

    fn on_panic(join_error: tokio::task::JoinError, _config: &Self, ctx: &ActorContext) {
        tracing::error!("persona rescan task panicked: {join_error}");
        let _ = ctx.send_event(Event::PersonasLoaded(PersonasLoaded {
            personas: vec![],
            error: Some(format!("rescan task failed: {join_error}")),
        }));
    }
}

/// Type alias for the persona scan actor.
pub type PersonaScanActor = ScanActor<PersonaScanConfig>;

// Re-export the old name for compatibility with actor_wiring.
// The tests from the original actor module are preserved below.

#[cfg(test)]
mod tests {
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

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_personas_command_emits_personas_loaded() {
        // Given an actor with a temp directory.
        let dir = tempfile::tempdir().expect("create temp dir");

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("persona-scan-test", sink.clone() as Arc<dyn MessageSink>);
        ctx.set_data(AppPaths::new_in(dir.path()));
        let mut actor = PersonaScanActor::activate(&mut ctx);

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

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("persona-scan-test", sink.clone() as Arc<dyn MessageSink>);
        ctx.set_data(AppPaths::new_in(dir.path()));
        let mut actor = PersonaScanActor::activate(&mut ctx);

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
        // Use paths that point to nonexistent dirs.
        let paths = AppPaths::new_in(dir.path());

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("persona-scan-test", sink.clone() as Arc<dyn MessageSink>);
        ctx.set_data(paths);
        let mut actor = PersonaScanActor::activate(&mut ctx);

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
