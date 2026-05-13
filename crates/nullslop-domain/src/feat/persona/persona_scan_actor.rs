//! Persona scanning actor.
//!
//! Subscribes to [`RescanPersonas`] commands, scans the personas
//! directory (path injected via [`ActorContext`] data), and emits
//! [`PersonasLoaded`] events with the results.

use std::path::PathBuf;
use std::sync::Arc;

use crate::common::actor::{
    Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink, SystemMessage,
};
use crate::common::actor_host::{ActorSpawnResult, spawn_actor_impl};
use crate::feat::context::protocol::command::RescanPersonas;
use crate::feat::context::protocol::event::PersonasLoaded;
use crate::feat::persona::scan_personas_dir;
use crate::protocol::{Command, Event};

/// Direct message type for the persona scan actor (unused).
pub enum PersonaScanDirectMsg {}

/// Persona scanning actor.
///
/// On `RescanPersonas`, scans the injected directory path, parses all
/// `*.md` files, and emits `PersonasLoaded` with the results.
///
/// The scan path is injected via [`ActorContext::set_data::<PathBuf>()`] during
/// actor spawn.
pub struct PersonaScanActor {
    /// Directory to scan for persona files.
    scan_path: PathBuf,
}

impl Actor for PersonaScanActor {
    type Message = PersonaScanDirectMsg;

    #[expect(
        clippy::expect_used,
        reason = "scan_path must be injected via ctx.set_data before activate"
    )]
    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<RescanPersonas>();
        let scan_path = ctx
            .take_data::<PathBuf>()
            .expect("PathBuf must be injected via ctx.set_data()");
        Self { scan_path }
    }

    async fn handle(&mut self, msg: ActorEnvelope<PersonaScanDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(command) => self.handle_command(&command, ctx).await,
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Event(_) | ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
}

impl PersonaScanActor {
    /// Dispatches incoming commands.
    async fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        match command {
            Command::RescanPersonas { .. } => {
                self.rescan(ctx).await;
            }
            _ => {}
        }
    }

    /// Scans the injected directory on a blocking thread and emits the result.
    async fn rescan(&self, ctx: &ActorContext) {
        let scan_path = self.scan_path.clone();
        let result = tokio::task::spawn_blocking(move || scan_personas_dir(&scan_path)).await;

        match result {
            Ok(personas) => {
                tracing::info!(count = personas.len(), "rescanned personas");
                let _ = ctx.send_event(Event::PersonasLoaded {
                    payload: PersonasLoaded {
                        personas,
                        error: None,
                    },
                });
            }
            Err(join_err) => {
                tracing::error!("persona rescan task panicked: {join_err}");
                let _ = ctx.send_event(Event::PersonasLoaded {
                    payload: PersonasLoaded {
                        personas: vec![],
                        error: Some(format!("rescan task failed: {join_err}")),
                    },
                });
            }
        }
    }
}

/// Spawns the persona scan actor on the given tokio runtime.
///
/// The persona scan actor scans and reloads persona files from the given directory.
pub fn spawn_persona_scan_actor(
    personas_dir: PathBuf,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<PersonaScanDirectMsg>, ActorSpawnResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<PersonaScanDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("persona-scan", sink);
    ctx.set_data(personas_dir);
    let actor = PersonaScanActor::activate(&mut ctx);
    let result = spawn_actor_impl("persona-scan", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
}
