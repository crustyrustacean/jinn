//! Prompt template scanning configuration for the generic scan actor.
//!
//! Subscribes to [`RescanPromptTemplates`] commands, scans the prompts
//! directory (path injected via [`ActorContext`] data), and emits
//! [`PromptTemplatesLoaded`] events with the results.

use std::path::Path;

use crate::common::actor::ActorContext;
use crate::common::actor::scan_actor::{ScanActor, ScanConfig};
use crate::feat::context::prompt_template::PromptTemplateStore;
use crate::feat::provider::protocol::command::RescanPromptTemplates;
use crate::feat::provider::protocol::event::PromptTemplatesLoaded;
use crate::protocol::{Command, Event};

/// Prompt template scan configuration for [`ScanActor`].
///
/// On `RescanPromptTemplates`, scans the injected directory path recursively,
/// parses all `*.md` files, and emits `PromptTemplatesLoaded` with the results.
///
/// The scan path is injected via [`ActorContext::set_data::<PathBuf>()`] during
/// actor spawn in `src/actor_wiring.rs`.
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

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<RescanPromptTemplates>();
        Self
    }

    fn is_rescan_command(command: &Command) -> bool {
        matches!(command, Command::RescanPromptTemplates)
    }

    fn scan(path: &Path) -> PromptScanResult {
        PromptTemplateStore::load_from_dir(path)
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
