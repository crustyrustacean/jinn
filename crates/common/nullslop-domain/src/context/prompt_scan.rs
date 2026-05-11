//! Prompt template scanning actor.
//!
//! Subscribes to [`RescanPromptTemplates`] commands, scans the prompts
//! directory (path injected via [`ActorContext`] data), and emits
//! [`PromptTemplatesLoaded`] events with the results.

use std::path::PathBuf;

use crate::actor::{Actor, ActorContext, ActorEnvelope, SystemMessage};
use crate::prompt_template::PromptTemplateStore;
use crate::protocol::provider::{PromptTemplatesLoaded, RescanPromptTemplates};
use crate::protocol::{Command, Event};

/// Direct message type for the prompt scan actor (unused).
pub enum PromptScanDirectMsg {}

/// Prompt template scanning actor.
///
/// On `RescanPromptTemplates`, scans the injected directory path recursively,
/// parses all `*.md` files, and emits `PromptTemplatesLoaded` with the results.
///
/// The scan path is injected via [`ActorContext::set_data::<PathBuf>()`] during
/// actor spawn in `src/app.rs`.
pub struct PromptScanActor {
    /// Directory to scan for prompt templates.
    scan_path: PathBuf,
}

impl Actor for PromptScanActor {
    type Message = PromptScanDirectMsg;

    #[expect(
        clippy::expect_used,
        reason = "scan_path must be injected via ctx.set_data before activate"
    )]
    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<RescanPromptTemplates>();
        let scan_path = ctx
            .take_data::<PathBuf>()
            .expect("PathBuf must be injected via ctx.set_data()");
        Self { scan_path }
    }

    async fn handle(&mut self, msg: ActorEnvelope<PromptScanDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(command) => self.handle_command(&command, ctx).await,
            ActorEnvelope::System(SystemMessage::ApplicationReady) => {
                ctx.announce_started();
            }
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Event(_) | ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
}

impl PromptScanActor {
    /// Dispatches incoming commands.
    async fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        match command {
            Command::RescanPromptTemplates => {
                self.rescan(ctx).await;
            }
            _ => {}
        }
    }

    /// Scans the injected directory on a blocking thread and emits the result.
    async fn rescan(&self, ctx: &ActorContext) {
        let scan_path = self.scan_path.clone();
        let result =
            tokio::task::spawn_blocking(move || PromptTemplateStore::load_from_dir(&scan_path))
                .await;

        match result {
            Ok(Ok(store)) => {
                tracing::info!(count = store.len(), "rescanned prompt templates");
                let _ = ctx.send_event(Event::PromptTemplatesLoaded {
                    payload: PromptTemplatesLoaded {
                        templates: store.templates().to_vec(),
                        error: None,
                    },
                });
            }
            Ok(Err(e)) => {
                tracing::warn!("failed to rescan prompt templates: {e:?}");
                let _ = ctx.send_event(Event::PromptTemplatesLoaded {
                    payload: PromptTemplatesLoaded {
                        templates: vec![],
                        error: Some(format!("{e:?}")),
                    },
                });
            }
            Err(join_err) => {
                tracing::error!("rescan task panicked: {join_err}");
                let _ = ctx.send_event(Event::PromptTemplatesLoaded {
                    payload: PromptTemplatesLoaded {
                        templates: vec![],
                        error: Some(format!("rescan task failed: {join_err}")),
                    },
                });
            }
        }
    }
}
