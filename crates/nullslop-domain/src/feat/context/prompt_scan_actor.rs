//! Prompt template scanning configuration for the generic scan actor.
//!
//! Subscribes to [`RescanPromptTemplates`] commands, scans user and system
//! prompts directories (paths injected via [`ActorContext`] data as [`AppPaths`]),
//! and emits [`PromptTemplatesLoaded`] events with the results.

use crate::common::actor::ActorContext;
use crate::common::actor::scan_actor::{ScanActor, ScanActorDeps, ScanConfig};
use crate::common::app_paths::AppPaths;
use crate::feat::context::prompt_template::PromptTemplateStore;
use crate::feat::provider::protocol::command::RescanPromptTemplates;
use crate::feat::provider::protocol::event::PromptTemplatesLoaded;
use crate::protocol::{Command, Event};

/// Prompt template scan configuration for [`ScanActor`].
///
/// On `RescanPromptTemplates`, scans both system and user prompt directories
/// recursively, parses all `*.md` files, and emits `PromptTemplatesLoaded`
/// with the merged results. User templates override system templates.
///
/// Paths are provided via [`ScanActorDeps`] during actor spawn in `src/actor_wiring.rs`.
pub struct PromptScanConfig;

/// The scan result from `PromptTemplateStore::load_from_dir` — may fail.
type PromptScanResult = Result<PromptTemplateStore, error_stack::Report<PromptTemplateStoreError>>;

/// Placeholder error type (unused — `load_from_dir` returns its own error type).
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct PromptTemplateLoadError;

use crate::feat::context::prompt_template::PromptTemplateStoreError;

impl ScanConfig for PromptScanConfig {
    type Output = PromptScanResult;

    fn activate(_deps: &ScanActorDeps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<RescanPromptTemplates>();
        Self
    }

    fn is_rescan_command(command: &Command) -> bool {
        matches!(command, Command::RescanPromptTemplates)
    }

    fn scan(paths: &AppPaths) -> PromptScanResult {
        PromptTemplateStore::load_from_dirs(&paths.prompts_dir(), &paths.system_prompts_dir())
    }

    fn on_success(result: PromptScanResult, _config: &Self, ctx: &ActorContext) {
        match result {
            Ok(store) => {
                tracing::info!(count = store.len(), "rescanned prompt templates");
                let _ = ctx.send_event(Event::PromptTemplatesLoaded(PromptTemplatesLoaded {
                    templates: store.templates().to_vec(),
                    error: None,
                }));
            }
            Err(e) => {
                tracing::warn!("failed to rescan prompt templates: {e:?}");
                let _ = ctx.send_event(Event::PromptTemplatesLoaded(PromptTemplatesLoaded {
                    templates: vec![],
                    error: Some(format!("{e:?}")),
                }));
            }
        }
    }

    fn on_panic(join_error: tokio::task::JoinError, _config: &Self, ctx: &ActorContext) {
        tracing::error!("rescan task panicked: {join_error}");
        let _ = ctx.send_event(Event::PromptTemplatesLoaded(PromptTemplatesLoaded {
            templates: vec![],
            error: Some(format!("rescan task failed: {join_error}")),
        }));
    }
}

/// Type alias for the prompt scan actor.
pub type PromptScanActor = ScanActor<PromptScanConfig>;
