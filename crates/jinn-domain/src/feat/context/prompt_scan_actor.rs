//! Prompt template scan actor — scans and reloads prompt templates on command.
//!
//! Subscribes to [`RescanPromptTemplates`] commands, scans user and system
//! prompts directories, and emits [`PromptTemplatesLoaded`] events with the results.

use crate::common::actor::scan_actor::NoDirectMsg;
use crate::common::actor::{Actor, ActorContext, ActorEnvelope};
use crate::common::app_paths::AppPaths;
use crate::feat::context::prompt_template::PromptTemplateStore;
use crate::feat::provider::protocol::command::RescanPromptTemplates;
use crate::feat::provider::protocol::event::PromptTemplatesLoaded;
use crate::protocol::{Command, Event};

/// Dependencies for [`PromptScanActor`].
pub struct PromptScanActorDeps {
    /// Application paths for resolving scan directories.
    pub paths: AppPaths,
}

/// Scans and reloads prompt templates on `RescanPromptTemplates`.
///
/// On command, scans both system and user prompt directories recursively,
/// parses all `*.md` files, and emits `PromptTemplatesLoaded` with the
/// merged results. User templates override system templates.
pub struct PromptScanActor {
    /// Application paths for resolving scan directories.
    paths: AppPaths,
}

impl Actor for PromptScanActor {
    type Message = NoDirectMsg;
    type Deps = PromptScanActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("Scans and reloads prompt templates");
        ctx.subscribe_command::<RescanPromptTemplates>();
        Self { paths: deps.paths }
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, ctx: &ActorContext) {
        if let ActorEnvelope::Command(command) = msg {
            self.handle_command(&command, ctx).await;
        }
    }
}

impl PromptScanActor {
    /// Dispatches incoming commands.
    async fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        if matches!(command, Command::RescanPromptTemplates) {
            self.run_scan(ctx).await;
        }
    }

    /// Runs the blocking scan and emits the result.
    async fn run_scan(&self, ctx: &ActorContext) {
        let paths = self.paths.clone();
        let result = tokio::task::spawn_blocking(move || {
            PromptTemplateStore::load_from_dirs(&paths.prompts_dir(), &paths.system_prompts_dir())
        })
        .await;

        match result {
            Ok(Ok(store)) => {
                tracing::info!(count = store.len(), "rescanned prompt templates");
                let _ = ctx.send_event(Event::PromptTemplatesLoaded(PromptTemplatesLoaded {
                    templates: store.templates().to_vec(),
                    error: None,
                }));
            }
            Ok(Err(e)) => {
                tracing::warn!("failed to rescan prompt templates: {e:?}");
                let _ = ctx.send_event(Event::PromptTemplatesLoaded(PromptTemplatesLoaded {
                    templates: vec![],
                    error: Some(format!("{e:?}")),
                }));
            }
            Err(join_error) => {
                tracing::error!("rescan task panicked: {join_error}");
                let _ = ctx.send_event(Event::PromptTemplatesLoaded(PromptTemplatesLoaded {
                    templates: vec![],
                    error: Some(format!("rescan task failed: {join_error}")),
                }));
            }
        }
    }
}
