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
use crate::feat::context::strategy::token_estimator::{CharRatioEstimator, TokenEstimator};
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
    /// Whether an auto-compaction request has already been dispatched
    /// and is awaiting processing. Prevents duplicate `EnqueueCompaction`
    /// commands from multiple `HistoryAppended` events within the same cycle.
    auto_compaction_pending: bool,
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
            auto_compaction_pending: false,
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
        let was_auto = self.auto_compaction_pending;
        self.auto_compaction_pending = false;

        tracing::info!(session_id = %cmd.session_id, "starting context compaction");

        let sink = ctx.sink();
        let session_id = cmd.session_id.clone();
        let state = self.state.clone();
        let services = self.services.clone();
        let rt_handle = self.handle.clone();
        let compact_all = cmd.compact_all;

        let task = rt_handle.clone().spawn(async move {
            let result = perform_compaction(
                &state,
                &services,
                &rt_handle,
                &session_id,
                &sink,
                was_auto,
                compact_all,
            )
            .await;

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
                        auto: was_auto,
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
                        auto: was_auto,
                        skipped: false,
                    }));
                    let _ = sink.send_event(Event::CompactionCompleted(CompactionCompleted {
                        session_id,
                        entries_compacted: 0,
                        auto: was_auto,
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
    fn handle_history_appended(&mut self, payload: &HistoryAppended, ctx: &ActorContext) {
        if self.auto_compaction_pending {
            tracing::debug!(
                session_id = ?payload.session_id,
                "skipping auto-compaction: already pending"
            );
            return;
        }

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

            let config = state.frontend.preferences.compaction.clone();

            // Resolve context window from the provider registry.
            // Falls back to `fallback_context_window` when the provider
            // doesn't report `context_length` (e.g., local models).
            let model_name = session.profile().model.clone();
            let provider_id = crate::feat::provider_infra::ProviderId::from(model_name);
            let context_window = self
                .services
                .provider_registry
                .get(&provider_id)
                .and_then(|r| r.context_length)
                .map_or(config.fallback_context_window, |c| c as usize);

            let total_tokens = payload.total_estimated_tokens;

            #[allow(clippy::cast_precision_loss)]
            let threshold_tokens = (config.threshold * context_window as f64) as usize;
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
            self.auto_compaction_pending = true;
            let _ = ctx.send_command(Command::EnqueueCompaction(EnqueueCompaction {
                session_id: session_id.clone(),
                compact_all: false,
            }));
            let _ = ctx.send_command(Command::SoftCancelTurn(
                crate::feat::session::protocol::soft_cancel_turn::SoftCancelTurn { session_id },
            ));
        }
    }

    /// Abort the in-flight compaction LLM task.
    fn handle_cancel_compaction(&mut self, payload: &CancelCompaction) {
        self.auto_compaction_pending = false;

        if let Some(task) = self.compaction_task.take() {
            task.abort();
            tracing::info!(session_id = %payload.session_id, "aborted compaction LLM task");
        }
    }
}

/// Adjust the token-based cut index forward to the next valid boundary.
///
/// The cut must not land on a `ToolCall` or `ToolResult` entry, because
/// these have structural dependencies on preceding messages:
///
/// - `ToolCall` merges into the preceding `Assistant` in `entries_to_messages`.
///   If that `Assistant` is compacted away, the tool call becomes orphaned.
/// - `ToolResult` produces a `tool` role message whose `tool_call_id` must
///   match a preceding `assistant.tool_calls[].id`. If the `Assistant` is
///   compacted but the `ToolResult` is kept, the provider rejects the request.
///
/// Walking forward past `ToolCall`/`ToolResult` to the next independent entry
/// (`Assistant`, `User`, `Error`, `System`, `Compaction`, etc.) ensures the
/// kept entries form a structurally valid LLM message sequence.
///
/// Returns the adjusted cut index (>= `cut_index`, <= `history.len()`).
fn adjust_cut_to_boundary(history: &[ChatEntry], cut_index: usize) -> usize {
    if cut_index >= history.len() {
        return cut_index;
    }

    // Pass 1: If cut lands on a ToolCall or ToolResult, walk forward past them.
    let cut_index = if matches!(
        history[cut_index].kind,
        ChatEntryKind::ToolCall { .. } | ChatEntryKind::ToolResult { .. }
    ) {
        history[cut_index..]
            .iter()
            .position(|entry| {
                !matches!(
                    entry.kind,
                    ChatEntryKind::ToolCall { .. } | ChatEntryKind::ToolResult { .. }
                )
            })
            .map_or(history.len(), |offset| cut_index + offset)
    } else {
        cut_index
    };

    if cut_index >= history.len() {
        return cut_index;
    }

    // Pass 2: If the cut lands on an Assistant, check for incomplete tool loops.
    // An incomplete tool loop has ToolCall entries without matching ToolResult entries.
    // Walking forward past them prevents dangling tool_calls in the kept entries.
    if !matches!(history[cut_index].kind, ChatEntryKind::Assistant(..)) {
        return cut_index;
    }

    // Scan the tool loop group starting from this Assistant.
    // Collect tool_call IDs and tool_result IDs within the group.
    let mut tool_call_ids: Vec<String> = Vec::new();
    let mut tool_result_ids: Vec<String> = Vec::new();
    let mut group_end = cut_index + 1;

    for (offset, entry) in history[cut_index + 1..].iter().enumerate() {
        match &entry.kind {
            ChatEntryKind::ToolCall { id, .. } => {
                tool_call_ids.push(id.clone());
                group_end = cut_index + 1 + offset + 1;
            }
            ChatEntryKind::ToolResult { id, .. } => {
                tool_result_ids.push(id.clone());
                group_end = cut_index + 1 + offset + 1;
            }
            // Stop at any entry that isn't part of this tool loop group.
            _ => break,
        }
    }

    // If all tool calls have matching results, the loop is complete — safe to cut here.
    if tool_call_ids.is_empty()
        || tool_call_ids
            .iter()
            .all(|id| tool_result_ids.iter().any(|r| r == id))
    {
        return cut_index;
    }

    // Incomplete tool loop — walk forward past the entire group and re-check.
    adjust_cut_to_boundary(history, group_end)
}

/// Compute the cut index by walking backwards accumulating tokens.
///
/// Returns the index where recent entries begin (those fitting within the reserve).
/// If `compact_all` is true, returns `history.len()`.
/// If all tokens fit within the reserve, returns `start_index`.
fn compute_cut_index(
    history: &[ChatEntry],
    start_index: usize,
    reserve_tokens: usize,
    compact_all: bool,
) -> usize {
    let estimator = CharRatioEstimator;

    if compact_all {
        return history.len();
    }

    let mut accumulated_tokens = 0usize;
    let mut cut_index = start_index;

    for i in (start_index..history.len()).rev() {
        let entry = &history[i];
        let tokens = estimate_entry_tokens(&estimator, entry);
        accumulated_tokens += tokens;
        if accumulated_tokens > reserve_tokens {
            cut_index = i + 1;
            break;
        }
    }

    cut_index
}

/// Gather compactable entries between `start_index` and `cut_index`,
/// excluding System and Compaction entries.
///
/// Returns `(gathered_indices, tokens_before)` where `tokens_before` is
/// the total estimated tokens of the gathered entries.
fn gather_compactable_entries(
    history: &[ChatEntry],
    start_index: usize,
    cut_index: usize,
) -> (Vec<usize>, usize) {
    let estimator = CharRatioEstimator;
    let mut gathered_indices: Vec<usize> = Vec::new();
    let mut tokens_before: usize = 0;
    for (i, entry) in history.iter().enumerate().take(cut_index).skip(start_index) {
        if matches!(entry.kind, ChatEntryKind::System(_)) || entry.is_compaction() {
            continue;
        }
        tokens_before += estimate_entry_tokens(&estimator, entry);
        gathered_indices.push(i);
    }
    (gathered_indices, tokens_before)
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
    auto: bool,
    compact_all: bool,
) -> Result<usize, error_stack::Report<CompactionError>> {
    // Read config and session state.
    let (config, model_name, history_len, retry_config, compaction_prompt) = {
        let state = state.read();
        let session = state.session(session_id);
        let config = state.frontend.preferences.compaction.clone();
        let model_name = session.profile().model.clone();
        let history_len = session.history().len();
        let retry_config = state.frontend.preferences.request_retry.to_retry_config();
        let compaction_prompt = state.context.compaction_prompt.clone();
        (config, model_name, history_len, retry_config, compaction_prompt)
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
        let cut_index = compute_cut_index(history, start_index, config.reserve_tokens, compact_all);
        let cut_index = adjust_cut_to_boundary(history, cut_index);

        // Gather entries from start to cut point, excluding System and Compaction.
        let (gathered_indices, tokens_before) = gather_compactable_entries(history, start_index, cut_index);

        if gathered_indices.is_empty() {
            // Nothing to compact — all tokens fit within the reserve.
            let total_tokens: usize = history[start_index..]
                .iter()
                .map(|e| estimate_entry_tokens(&CharRatioEstimator, e))
                .sum();

            let skip_msg = format!(
                "Skipped compaction: {} tokens within the {} token reserve. Use /compact-all to force.",
                total_tokens, config.reserve_tokens
            );

            // Emit BeginCompaction so session enters Compacting phase.
            let _ = sink.send_command(Command::BeginCompaction(BeginCompaction {
                session_id: session_id.clone(),
                gathered_indices: vec![],
            }));
            // Emit EndCompaction with skipped flag so session shows message and returns to Idle.
            let _ = sink.send_command(Command::EndCompaction(EndCompaction {
                session_id: session_id.clone(),
                result: None,
                error: Some(skip_msg),
                auto,
                skipped: true,
            }));

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
        services,
        handle,
        &serialized,
        previous_summary.as_deref(),
        &model_name,
        &config,
        &retry_config,
        &compaction_prompt,
    )
    .await?;

    // Step 4: Emit EndCompaction — session actor inserts entry, sets phase to Idle.
    let tokens_after = CharRatioEstimator.estimate(&summary);
    let _ = sink.send_command(Command::EndCompaction(EndCompaction {
        session_id: session_id.clone(),
        result: Some(CompactionResult {
            summary,
            entries_compacted,
            tokens_before,
            tokens_after,
            model_used: model_name.clone(),
            boundary_index,
        }),
        error: None,
        auto,
        skipped: false,
    }));

    Ok(entries_compacted)
}

/// Generate a summary using the LLM.
#[allow(clippy::too_many_arguments)]
async fn generate_summary(
    services: &Services,
    runtime_handle: &Handle,
    serialized_entries: &str,
    previous_summary: Option<&str>,
    session_model: &str,
    config: &CompactionConfig,
    retry_config: &nullslop_provider::RetryConfig,
    compaction_prompt: &str,
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

    let system_prompt = compaction_prompt.to_owned();

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
