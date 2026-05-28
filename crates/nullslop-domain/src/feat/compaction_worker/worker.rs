//! Compaction worker — summarizes conversation history into structured checkpoints.
//!
//! Implements [`HistoryWorker`] to produce [`HistoryMutation`] batches that
//! exclude old entries and insert a compaction summary. Runs asynchronously
//! (LLM call for summarization).

use error_stack::ResultExt as _;
use futures::{StreamExt, pin_mut};
use nullslop_provider::LlmMessage;
use nullslop_provider::LlmService;
use nullslop_provider::{NoOpOnRetry, RetryingLlmService};
use tokio::runtime::Handle;
use wherror::Error;

use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::compaction_worker::serializer::serialize_entries_for_compaction;
use crate::feat::compaction_worker::algorithm::{
    adjust_cut_to_boundary, compute_cut_index, find_start_boundary, gather_compactable_entries,
};
use crate::feat::context::strategy::token_estimator::{CharRatioEstimator, TokenEstimator};

use crate::feat::preferences_actor::user_preferences::CompactionConfig;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryId, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::protocol::SessionId;

/// Errors during compaction.
#[derive(Debug, Error)]
#[error("compaction error")]
pub struct CompactionError;

/// Compaction trigger for the worker.
#[derive(Debug, Clone)]
pub struct CompactionTrigger {
    /// The session to compact.
    pub session_id: SessionId,
    /// Whether to compact all entries (force, ignoring reserve).
    pub compact_all: bool,
}

/// The compaction worker.
///
/// Evaluates session history, determines which entries to compact,
/// calls the LLM for summarization, and produces mutations.
#[derive(Clone)]
pub struct CompactionWorker {
    /// Runtime services (for LLM calls).
    pub services: Services,
    /// Tokio runtime handle for spawning tasks.
    pub handle: Handle,
    /// Shared application state (for reading session history).
    pub state: State,
    /// Compaction configuration.
    pub config: CompactionConfig,
    /// System prompt for compaction summarization.
    pub compaction_prompt: String,
}

#[async_trait::async_trait]
impl HistoryWorker for CompactionWorker {
    fn name(&self) -> &str {
        "compaction"
    }

    async fn evaluate(&self, history: Vec<ChatEntry>) -> Vec<HistoryMutation> {
        // Delegate to the full evaluate_for_session which needs state access.
        // The history worker actor provides the history snapshot.
        // We need to get config and session-specific data separately.
        self.evaluate_history(&history).await
    }
}

impl CompactionWorker {
    /// Full evaluation with session state access.
    pub async fn evaluate_for_session(&self, trigger: &CompactionTrigger) -> Vec<HistoryMutation> {
        // Read config and session state.
        let (config, model_name, history, compaction_prompt, retry_config) = {
            let state = self.state.read();
            let session = state.session(&trigger.session_id);
            let config = state.frontend.preferences.compaction.clone();
            let model_name = session.profile().model.clone();
            let history = session.history().to_vec();
            let compaction_prompt = state.context.compaction_prompt.clone();
            let retry_config = state.frontend.preferences.request_retry.to_retry_config();
            (config, model_name, history, compaction_prompt, retry_config)
        };

        if history.is_empty() {
            return vec![];
        }

        let result = self
            .evaluate_with_config(
                &history,
                &config,
                &model_name,
                &compaction_prompt,
                &retry_config,
                trigger.compact_all,
            )
            .await;

        match result {
            Ok(mutations) => mutations,
            Err(e) => {
                tracing::error!(
                    session_id = %trigger.session_id,
                    error = %e,
                    "compaction worker evaluation failed"
                );
                vec![]
            }
        }
    }

    /// Evaluate history using just the history snapshot (for HistoryWorker trait).
    async fn evaluate_history(&self, history: &[ChatEntry]) -> Vec<HistoryMutation> {
        // For the trait-based path, we use stored config.
        let (config, model_name, compaction_prompt, retry_config) = {
            let state = self.state.read();
            let config = self.config.clone();
            // Use model from the first session found, or fallback.
            let model_name = state
                .session
                .iter()
                .next()
                .map(|(_, s)| s.profile().model.clone())
                .unwrap_or_default();
            let compaction_prompt = state.context.compaction_prompt.clone();
            let retry_config = state.frontend.preferences.request_retry.to_retry_config();
            (config, model_name, compaction_prompt, retry_config)
        };

        let result = self
            .evaluate_with_config(
                history,
                &config,
                &model_name,
                &compaction_prompt,
                &retry_config,
                false, // Not compact_all for auto-trigger
            )
            .await;

        match result {
            Ok(mutations) => mutations,
            Err(e) => {
                tracing::error!(error = %e, "compaction worker evaluation failed");
                vec![]
            }
        }
    }

    /// Core evaluation algorithm.
    #[allow(clippy::too_many_lines)]
    async fn evaluate_with_config(
        &self,
        history: &[ChatEntry],
        config: &CompactionConfig,
        model_name: &str,
        compaction_prompt: &str,
        retry_config: &nullslop_provider::RetryConfig,
        compact_all: bool,
    ) -> Result<Vec<HistoryMutation>, error_stack::Report<CompactionError>> {
        // Step 1: Find start boundary.
        let start_index = find_start_boundary(history);

        // Step 2: Compute cut index.
        let cut_index = compute_cut_index(history, start_index, config.reserve_tokens, compact_all);
        let cut_index = adjust_cut_to_boundary(history, cut_index);

        // Step 3: Gather compactable entries.
        let (gathered_indices, tokens_before) =
            gather_compactable_entries(history, start_index, cut_index);

        if gathered_indices.is_empty() {
            // Nothing to compact — all tokens fit within the reserve.
            return Ok(vec![]);
        }

        // Step 4: Collect entries for serialization.
        let entries_to_serialize: Vec<ChatEntry> = gathered_indices
            .iter()
            .map(|&i| history[i].clone())
            .collect();

        // Step 5: Check for previous compaction summary.
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

        let entries_compacted = gathered_indices.len();

        // Step 6: Serialize entries for LLM.
        let serialized: String = tokio::task::spawn_blocking(move || {
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

        // Step 7: Call LLM for summarization.
        let summary = generate_summary(
            &self.services,
            &self.handle,
            &serialized,
            previous_summary.as_deref(),
            model_name,
            config,
            retry_config,
            compaction_prompt,
        )
        .await?;

        // Step 8: Produce mutations.
        let mut mutations = Vec::new();

        // 8a: Set ForcedExclude for each gathered entry.
        for &idx in &gathered_indices {
            let entry = &history[idx];
            mutations.push(HistoryMutation::SetContextOverride {
                entry_id: entry.id.clone(),
                value: ContextOverride::ForcedExclude,
            });
        }

        // 8b: Insert compaction summary entry after the last gathered entry.
        let tokens_after = CharRatioEstimator.estimate(&summary);
        let compaction_entry = ChatEntry {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::Compaction {
                summary,
                tokens_before,
                tokens_after,
                entries_compacted,
                model_used: model_name.to_owned(),
            },
            pin_position: None,
            context_override: ContextOverride::Default,
        };

        let last_gathered_id = gathered_indices
            .last()
            .map(|&idx| history[idx].id.clone());
        mutations.push(HistoryMutation::InsertEntry {
            after_entry_id: last_gathered_id,
            entry: compaction_entry,
        });

        Ok(mutations)
    }
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
                services.llm_service.create()
            })
            .change_context(CompactionError)
            .attach("failed to create LLM service for compaction")?
    };

    // Wrap with retry decorator.
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
