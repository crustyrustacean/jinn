//! Compaction actor — summarizes conversation history into structured checkpoints.

pub mod protocol;
pub mod serializer;

mod mod_tests;

pub use protocol::command::CompactContext;
pub use protocol::event::CompactionCompleted;

use std::sync::Arc;

use error_stack::ResultExt as _;
use futures::{StreamExt, pin_mut};
use nullslop_provider::LlmMessage;
use nullslop_provider::LlmService;
use tokio::runtime::Handle;
use wherror::Error;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::compaction_actor::serializer::serialize_entries_for_compaction;
use crate::feat::context::strategy::compaction_prompt::load_compaction_prompt;
use crate::feat::context::strategy::token_estimator::{
    CharRatioEstimator, TokenEstimator, estimate_entry_tokens,
};
use crate::feat::preferences_actor::user_preferences::CompactionConfig;
use crate::feat::provider::protocol::event::StreamCompleted;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};
use crate::protocol::{ChatEntryId, Command, Event};

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
}

impl Actor for CompactionActor {
    type Message = NoDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<CompactContext>();
        ctx.subscribe_event::<StreamCompleted>();

        let state = ctx
            .take_data::<State>()
            .expect("State must be injected before activate");
        let services = ctx
            .take_data::<Services>()
            .expect("Services must be injected before activate");
        let handle = ctx
            .take_data::<Handle>()
            .expect("Handle must be injected before activate");

        Self {
            state,
            services,
            handle,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(cmd) => {
                if let Command::CompactContext(ref payload) = cmd {
                    self.handle_compact_context(payload, ctx).await;
                }
            }
            ActorEnvelope::Event(event) => {
                if let Event::StreamCompleted(ref payload) = event {
                    self.handle_stream_completed(payload, ctx);
                }
            }
            ActorEnvelope::System(_) => {}
            _ => {}
        }
    }
}

impl CompactionActor {
    /// Handle a `CompactContext` command.
    async fn handle_compact_context(&self, cmd: &CompactContext, ctx: &ActorContext) {
        tracing::info!(session_id = %cmd.session_id, "starting context compaction");

        let result = self.perform_compaction(cmd).await;

        match result {
            Ok(entries_compacted) => {
                tracing::info!(
                    session_id = %cmd.session_id,
                    entries_compacted,
                    "context compaction completed"
                );
                let _ = ctx.send_event(Event::CompactionCompleted(CompactionCompleted {
                    session_id: cmd.session_id.clone(),
                    entries_compacted,
                }));
            }
            Err(e) => {
                tracing::error!(
                    session_id = %cmd.session_id,
                    error = %e,
                    "context compaction failed"
                );
                // Emit completion with 0 to unblock any waiting logic.
                let _ = ctx.send_event(Event::CompactionCompleted(CompactionCompleted {
                    session_id: cmd.session_id.clone(),
                    entries_compacted: 0,
                }));
            }
        }
    }

    /// Handle a `StreamCompleted` event for auto-trigger.
    ///
    /// Only fires on `Finished` reason. Estimates total tokens and compares
    /// against `threshold * token_budget`. If exceeded, sends `CompactContext`.
    fn handle_stream_completed(&self, payload: &StreamCompleted, ctx: &ActorContext) {
        use crate::feat::provider::protocol::event::StreamCompletedReason;

        // Only trigger on normal completion, not tool use or errors.
        if payload.reason != StreamCompletedReason::Finished {
            return;
        }

        let (should_compact, session_id) = {
            let state = self.state.read();
            let config = &state.frontend.preferences.compaction;
            let token_budget = state.frontend.preferences.context_token_budget.budget;
            let session_id = payload.session_id.clone();
            let session = state.session(&session_id);

            let estimator = CharRatioEstimator;
            let total_tokens: usize = session
                .history()
                .iter()
                .map(|e| estimate_entry_tokens(&estimator, e))
                .sum();

            let threshold_tokens = (config.threshold * token_budget as f64) as usize;
            let should = total_tokens > threshold_tokens;

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
            let _ = ctx.send_command(Command::CompactContext(CompactContext { session_id }));
        }
    }

    /// Perform the compaction algorithm.
    async fn perform_compaction(
        &self,
        cmd: &CompactContext,
    ) -> Result<usize, error_stack::Report<CompactionError>> {
        // Read config and session state.
        let (config, model_name, history_len) = {
            let state = self.state.read();
            let session = state.session(&cmd.session_id);
            let config = state.frontend.preferences.compaction.clone();
            let model_name = session.profile().model.clone();
            let history_len = session.history().len();
            (config, model_name, history_len)
        };

        if history_len == 0 {
            return Ok(0);
        }

        // Step 1: Find start boundary, cut point, gather entries, mark ignored.
        let gathered = {
            let mut state = self.state.write();
            let session = state.session_mut(&cmd.session_id);

            let history = session.history();

            // Find the start boundary: index after the last Compaction entry.
            let start_index = history
                .iter()
                .rposition(|e| e.is_compaction())
                .map(|i| i + 1)
                .unwrap_or(0);

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
            for i in start_index..cut_index {
                let entry = &history[i];
                if matches!(entry.kind, ChatEntryKind::System(_)) || entry.is_compaction() {
                    continue;
                }
                tokens_before += estimate_entry_tokens(&estimator, entry);
                gathered_indices.push(i);
            }

            if gathered_indices.is_empty() {
                return Ok(0);
            }

            // Serialize before marking as ignored.
            let entries_to_serialize: Vec<ChatEntry> = gathered_indices
                .iter()
                .map(|&i| history[i].clone())
                .collect();
            let serialized = serialize_entries_for_compaction(&entries_to_serialize);

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

            // Mark all gathered entries as ignored.
            let entries_compacted = gathered_indices.len();
            session.mark_entries_ignored(&gathered_indices);

            // Determine boundary insertion point: right after the last gathered entry.
            let boundary_index = gathered_indices
                .last()
                .expect("gathered_indices is non-empty")
                + 1;

            (
                serialized,
                previous_summary,
                entries_compacted,
                tokens_before,
                boundary_index,
            )
        };

        let (serialized, previous_summary, entries_compacted, tokens_before, boundary_index) =
            gathered;

        // Step 2: Call LLM for summarization.
        let summary = self
            .generate_summary(
                &serialized,
                previous_summary.as_deref(),
                &model_name,
                &config,
            )
            .await?;

        // Step 3: Insert Compaction entry and push System entry.
        {
            let mut state = self.state.write();
            let session = state.session_mut(&cmd.session_id);

            let compaction_entry = ChatEntry {
                id: ChatEntryId::new(),
                timestamp: jiff::Timestamp::now(),
                kind: ChatEntryKind::Compaction {
                    summary,
                    tokens_before,
                    entries_compacted,
                    model_used: model_name.clone(),
                },
                pin_position: None,
                ignored: false,
            };

            session.insert_entry_at(boundary_index, compaction_entry);

            // Push a system message about the compaction.
            session.push_entry(ChatEntry::system(format!(
                "Context was compacted. {entries_compacted} messages were summarized."
            )));
        }

        Ok(entries_compacted)
    }

    /// Generate a summary using the LLM.
    async fn generate_summary(
        &self,
        serialized_entries: &str,
        previous_summary: Option<&str>,
        session_model: &str,
        config: &CompactionConfig,
    ) -> Result<String, error_stack::Report<CompactionError>> {
        // Try compaction model first, fall back to session model.
        let model_id = config.model.as_deref().unwrap_or(session_model);

        // Create LLM service via provider registry.
        let provider_id = crate::feat::provider_infra::ProviderId::from(model_id.to_owned());
        let service: Box<dyn LlmService> = {
            let api_keys = self.services.api_keys.read();
            self.services
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
                    self.services.llm_service.create()
                })
                .change_context(CompactionError)
                .attach("failed to create LLM service for compaction")?
        };

        // Build the prompt.
        let system_prompt = {
            let config_dir = self
                .services
                .paths
                .preferences_path()
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .to_path_buf();
            load_compaction_prompt(&config_dir)
        };

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
                return Err(error_stack::Report::new(CompactionError)
                    .attach("compaction LLM stream failed"));
            }
        };

        let handle = self.handle.clone();
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
}
