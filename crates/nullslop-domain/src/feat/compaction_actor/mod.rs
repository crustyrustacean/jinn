//! Compaction actor — summarizes conversation history into structured checkpoints.

pub mod protocol;
pub mod serializer;

#[cfg(test)]
mod mod_tests;

pub use protocol::command::{CompactContext, EnqueueCompaction};
pub use protocol::event::CompactionCompleted;

use error_stack::ResultExt as _;
use futures::{StreamExt, pin_mut};
use nullslop_provider::LlmMessage;
use nullslop_provider::LlmService;
use nullslop_provider::{NoOpOnRetry, RetryingLlmService};
use tokio::runtime::Handle;
use wherror::Error;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::compaction_actor::protocol::command::{
    BeginCompaction, CancelCompaction, CompactionResult, EndCompaction,
};
use crate::feat::compaction_actor::serializer::serialize_entries_for_compaction;
use crate::feat::context::strategy::token_estimator::CharRatioEstimator;
use crate::feat::context::strategy::token_estimator::estimate_entry_tokens;
use crate::feat::preferences_actor::user_preferences::CompactionConfig;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};
use crate::feat::session::chat_session::SessionPhase;
use crate::feat::session::protocol::history_appended::HistoryAppended;
use crate::protocol::{Command, Event};

/// Errors during compaction.
#[derive(Debug, Error)]
#[error("compaction error")]
pub struct CompactionError;

/// The compaction actor.
///
/// Subscribes to [`CompactContext`] commands and performs context compaction.
/// Uses a configurable LLM model for summarization (falls back to session model).
pub struct CompactionActor {
    state: State,
    services: Services,
    handle: Handle,
    /// `JoinHandle` for the in-flight compaction LLM task.
    /// `Some` while a compaction is in progress, `None` otherwise.
    compaction_task: Option<tokio::task::JoinHandle<()>>,
}

/// Dependencies for [`CompactionActor`].
pub struct CompactionActorDeps {
    /// Shared application state.
    pub state: State,
    /// Runtime services.
    pub services: Services,
    /// Tokio runtime handle for spawning tasks.
    pub handle: Handle,
}

impl Actor for CompactionActor {
    type Message = NoDirectMsg;
    type Deps = CompactionActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("Summarizes conversation history into structured checkpoints");
        ctx.subscribe_command::<CompactContext>();
        ctx.subscribe_command::<CancelCompaction>();
        ctx.subscribe_event::<HistoryAppended>();

        Self {
            state: deps.state,
            services: deps.services,
            handle: deps.handle,
            compaction_task: None,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(Command::CompactContext(ref payload)) => {
                self.handle_compact_context(payload, ctx);
            }
            ActorEnvelope::Event(Event::HistoryAppended(ref payload)) => {
                self.handle_history_appended(payload, ctx);
            }
            ActorEnvelope::Command(Command::CancelCompaction(ref payload)) => {
                self.handle_cancel_compaction(payload);
            }
            ActorEnvelope::Command(_)
            | ActorEnvelope::Event(_)
            | ActorEnvelope::System(_)
            | ActorEnvelope::Direct(_) => {}
        }
    }
}

impl CompactionActor {
    /// Handle a `CompactContext` command.
    ///
    /// Spawns the compaction flow as a background task so the actor
    /// can continue processing messages (e.g. `CancelCompaction`).
    ///
    /// The spawned task:
    /// 1. Emits `BeginCompaction` — marks entries ignored, sets phase to Compacting
    /// 2. Calls the LLM for summarization
    /// 3. Emits `EndCompaction` — inserts result entry, sets phase to Idle
    /// 4. Emits `CompactionCompleted` event — signals persistence
    fn handle_compact_context(&mut self, cmd: &CompactContext, ctx: &ActorContext) {
        tracing::info!(session_id = %cmd.session_id, "starting context compaction");

        let sink = ctx.sink();
        let session_id = cmd.session_id.clone();
        let state = self.state.clone();
        let services = self.services.clone();
        let rt_handle = self.handle.clone();

        let task = rt_handle.clone().spawn(async move {
            let result =
                perform_compaction(&state, &services, &rt_handle, &session_id, &sink).await;

            match result {
                Ok(entries_compacted) => {
                    tracing::info!(
                        session_id = %session_id,
                        entries_compacted,
                        "context compaction completed"
                    );
                    let _ = sink.send_event(Event::CompactionCompleted(CompactionCompleted {
                        session_id: session_id.clone(),
                        entries_compacted,
                    }));
                }
                Err(e) => {
                    tracing::error!(
                        session_id = %session_id,
                        error = %e,
                        "context compaction failed"
                    );
                    let _ = sink.send_command(Command::EndCompaction(EndCompaction {
                        session_id: session_id.clone(),
                        result: None,
                        error: Some(format!("{e:#}")),
                    }));
                    let _ = sink.send_event(Event::CompactionCompleted(CompactionCompleted {
                        session_id,
                        entries_compacted: 0,
                    }));
                }
            }
        });

        self.compaction_task = Some(task);
    }

    /// Handle a `HistoryAppended` event for auto-compaction trigger.
    ///
    /// Compares the reported `total_estimated_tokens` against the configured
    /// threshold. If exceeded, sends `EnqueueCompaction` (to queue compaction)
    /// and `SoftCancelTurn` (to gracefully end the current turn if any).
    fn handle_history_appended(&self, payload: &HistoryAppended, ctx: &ActorContext) {
        let (should_compact, session_id) = {
            let state = self.state.read();
            let session_id = payload.session_id.clone();
            let session = state.session(&session_id);

            // Don't auto-trigger if already compacting.
            if matches!(session.phase(), SessionPhase::Compacting) {
                tracing::debug!(
                    session_id = ?session_id,
                    "skipping auto-compaction: session is already compacting"
                );
                return;
            }

            let config = &state.frontend.preferences.compaction;
            let token_budget = state.frontend.preferences.context_token_budget.budget;

            let total_tokens = payload.total_estimated_tokens;

            #[allow(clippy::cast_precision_loss)]
            let threshold_tokens = (config.threshold * token_budget as f64) as usize;
            let should = total_tokens > threshold_tokens;

            tracing::debug!(
                session_id = ?session_id,
                total_tokens,
                threshold_tokens,
                should,
                "auto-compaction threshold evaluation"
            );

            if should {
                tracing::info!(
                    session_id = ?session_id,
                    total_tokens,
                    threshold_tokens,
                    "auto-compaction threshold exceeded"
                );
            }

            (should, session_id)
        };

        if should_compact {
            let _ = ctx.send_command(Command::EnqueueCompaction(EnqueueCompaction { session_id: session_id.clone() }));
            let _ = ctx.send_command(Command::SoftCancelTurn(
                crate::feat::session::protocol::soft_cancel_turn::SoftCancelTurn { session_id },
            ));
        }
    }

    /// Abort the in-flight compaction LLM task.
    fn handle_cancel_compaction(&mut self, payload: &CancelCompaction) {
        if let Some(task) = self.compaction_task.take() {
            task.abort();
            tracing::info!(session_id = %payload.session_id, "aborted compaction LLM task");
        }
    }
}

/// Perform the compaction algorithm as a free async function.
///
/// This runs inside a spawned task so the actor is free to process
/// other messages (e.g. `CancelCompaction`).
///
/// This function only reads state — all writes go through commands
/// emitted via the `sink`.
#[allow(clippy::too_many_lines)]
async fn perform_compaction(
    state: &State,
    services: &Services,
    handle: &Handle,
    session_id: &crate::protocol::SessionId,
    sink: &std::sync::Arc<dyn crate::common::actor::message_sink::MessageSink>,
) -> Result<usize, error_stack::Report<CompactionError>> {
    // Read config and session state.
    let (config, model_name, history_len, retry_config) = {
        let state = state.read();
        let session = state.session(session_id);
        let config = state.frontend.preferences.compaction.clone();
        let model_name = session.profile().model.clone();
        let history_len = session.history().len();
        let retry_config = state.frontend.preferences.request_retry.to_retry_config();
        (config, model_name, history_len, retry_config)
    };

    if history_len == 0 {
        return Ok(0);
    }

    // Step 1: Read-only — find start boundary, cut point, gather entries.
    let gathered = {
        let state = state.read();
        let session = state.session(session_id);

        let history = session.history();

        // Find the start boundary: index after the last Compaction entry.
        let start_index = history
            .iter()
            .rposition(ChatEntry::is_compaction)
            .map_or(0, |i| i + 1);

        // Find cut point: walk backwards accumulating tokens.
        let estimator = CharRatioEstimator;
        let mut accumulated_tokens = 0usize;
        let mut cut_index = history.len(); // Default: compact everything after start.

        for i in (start_index..history.len()).rev() {
            let entry = &history[i];
            let tokens = estimate_entry_tokens(&estimator, entry);
            accumulated_tokens += tokens;
            if accumulated_tokens > config.keep_recent_tokens {
                cut_index = i + 1;
                break;
            }
        }

        // Gather entries from start to cut point, excluding System and Compaction.
        let mut gathered_indices: Vec<usize> = Vec::new();
        let mut tokens_before: usize = 0;
        for (i, entry) in history.iter().enumerate().take(cut_index).skip(start_index) {
            if matches!(entry.kind, ChatEntryKind::System(_)) || entry.is_compaction() {
                continue;
            }
            tokens_before += estimate_entry_tokens(&estimator, entry);
            gathered_indices.push(i);
        }

        if gathered_indices.is_empty() {
            return Ok(0);
        }

        // Collect entries for serialization.
        let entries_to_serialize: Vec<ChatEntry> = gathered_indices
            .iter()
            .map(|&i| history[i].clone())
            .collect();

        // Check for previous compaction summary (for iterative updating).
        let previous_summary = if start_index > 0 {
            let prev = &history[start_index - 1];
            if let ChatEntryKind::Compaction { summary, .. } = &prev.kind {
                Some(summary.clone())
            } else {
                None
            }
        } else {
            None
        };

        // Determine boundary insertion point: right after the last gathered entry.
        let Some(&last_index) = gathered_indices.last() else {
            return Ok(0);
        };
        let boundary_index = last_index + 1;

        let entries_compacted = gathered_indices.len();

        (
            entries_to_serialize,
            previous_summary,
            entries_compacted,
            tokens_before,
            boundary_index,
            gathered_indices,
        )
    };

    let (
        entries_to_serialize,
        previous_summary,
        entries_compacted,
        tokens_before,
        boundary_index,
        gathered_indices,
    ) = gathered;

    // CPU-bound: serialize entries — offloaded to blocking thread.
    let serialized = tokio::task::spawn_blocking(move || {
        serialize_entries_for_compaction(&entries_to_serialize)
    })
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(
            err = ?e,
            "spawn_blocking panicked during compaction serialization"
        );
        String::new()
    });

    // Step 2: Emit BeginCompaction — session actor marks entries ignored, sets phase.
    let _ = sink.send_command(Command::BeginCompaction(BeginCompaction {
        session_id: session_id.clone(),
        gathered_indices,
    }));

    // Step 3: Call LLM for summarization.
    let summary = generate_summary(
        state,
        services,
        handle,
        &serialized,
        previous_summary.as_deref(),
        &model_name,
        &config,
        &retry_config,
    )
    .await?;

    // Step 4: Emit EndCompaction — session actor inserts entry, sets phase to Idle.
    let _ = sink.send_command(Command::EndCompaction(EndCompaction {
        session_id: session_id.clone(),
        result: Some(CompactionResult {
            summary,
            entries_compacted,
            tokens_before,
            model_used: model_name.clone(),
            boundary_index,
        }),
        error: None,
    }));

    Ok(entries_compacted)
}

/// Generate a summary using the LLM.
#[allow(clippy::too_many_arguments)]
async fn generate_summary(
    state: &State,
    services: &Services,
    runtime_handle: &Handle,
    serialized_entries: &str,
    previous_summary: Option<&str>,
    session_model: &str,
    config: &CompactionConfig,
    retry_config: &nullslop_provider::RetryConfig,
) -> Result<String, error_stack::Report<CompactionError>> {
    // Try compaction model first, fall back to session model.
    let model_id = config.model.as_deref().unwrap_or(session_model);

    // Create LLM service via provider registry.
    let provider_id = crate::feat::provider_infra::ProviderId::from(model_id.to_owned());
    let service: Box<dyn LlmService> = {
        let api_keys = services.api_keys.read();
        services
            .provider_registry
            .create_factory(&provider_id, &api_keys)
            .and_then(|factory| factory.create())
            .or_else(|e| {
                tracing::warn!(
                    model = model_id,
                    error = %e,
                    "failed to create compaction model, falling back to session model"
                );
                // Fall back to session model via the shared factory.
                services.llm_service.create()
            })
            .change_context(CompactionError)
            .attach("failed to create LLM service for compaction")?
    };

    // Wrap with retry decorator — compaction retries are logged at warn level.
    let service = RetryingLlmService::new(service, retry_config.clone(), Box::new(NoOpOnRetry));

    // Build the prompt — prefer template from store, fall back to bundled default.
    let system_prompt = state
        .read()
        .context
        .prompt_templates
        .find_by_name("compaction")
        .map_or_else(
            || {
                crate::feat::context::strategy::compaction_prompt::DEFAULT_COMPACTION_PROMPT
                    .to_owned()
            },
            |t| t.body.clone(),
        );

    let mut user_content = String::new();
    if let Some(prev) = previous_summary {
        user_content.push_str("A previous compaction summary exists:\n\n<previous-summary>\n");
        user_content.push_str(prev);
        user_content.push_str("\n</previous-summary>\n\n");
        user_content.push_str(
            "The messages below are NEW conversation messages to incorporate into the existing summary. \
             Update the existing structured summary to reflect the new information.\n\n",
        );
    } else {
        user_content.push_str(
            "The messages below are a conversation to summarize. \
             Create a structured context checkpoint summary.\n\n",
        );
    }
    user_content.push_str(serialized_entries);

    let messages = vec![
        LlmMessage::System {
            content: system_prompt,
        },
        LlmMessage::User {
            content: user_content,
        },
    ];

    // Call the LLM and collect the response.
    let stream = match service.chat_stream(messages).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to start compaction LLM stream");
            return Err(
                error_stack::Report::new(CompactionError).attach("compaction LLM stream failed")
            );
        }
    };

    let handle = runtime_handle.clone();
    let summary = handle
        .spawn(async move {
            let mut full_response = String::new();
            pin_mut!(stream);
            while let Some(result) = stream.next().await {
                match result {
                    Ok(token) => full_response.push_str(&token),
                    Err(e) => {
                        tracing::warn!(error = %e, "error during compaction LLM stream");
                        break;
                    }
                }
            }
            full_response
        })
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "compaction LLM task panicked");
            error_stack::Report::new(CompactionError).attach("compaction LLM task panicked")
        })?;

    Ok(summary)
}
