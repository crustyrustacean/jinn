//! Compaction worker - summarizes conversation history into brief checkpoints.
//!
//! Implements [`HistoryWorker`] to produce [`HistoryMutation`] batches that
//! exclude old entries and insert a compaction summary. Runs asynchronously
//! (LLM call for summarization).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use error_stack::ResultExt as _;
use futures::{StreamExt, pin_mut};
use jinn_provider::LlmMessage;
use jinn_provider::LlmService;
use jinn_provider::{NoOpOnRetry, RetryingLlmService};
use tokio::runtime::Handle;
use wherror::Error;

use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::compaction_worker::algorithm::{
    adjust_cut_to_boundary, compute_cut_index, find_start_boundary, gather_compactable_entries,
};
use crate::feat::compaction_worker::serializer::serialize_entries_for_compaction;
use crate::feat::context::strategy::token_estimator::{CharRatioEstimator, TokenEstimator};

use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::preferences_actor::user_preferences::CompactionConfig;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryId, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::protocol::SessionId;

/// Errors during compaction.
#[derive(Debug, Error)]
#[error("compaction error")]
pub struct CompactionError;

/// Maximum estimated tokens for a compaction summary.
///
/// The streaming collection loop stops accepting tokens once this ceiling
/// is reached, preventing runaway summaries (e.g., 25k+ tokens).
const MAX_SUMMARY_TOKENS: usize = 4000;

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
    /// Whether an auto-compaction is currently in flight.
    ///
    /// Set before starting the LLM call, cleared when the compaction
    /// summary entry is found in a subsequent snapshot (meaning mutations
    /// were applied) or on error.
    pub(crate) compaction_in_progress: Arc<AtomicBool>,
    /// The `ChatEntryId` of the pending compaction summary entry.
    ///
    /// Used to detect when mutations have been applied by scanning the snapshot.
    /// `None` when no compaction is in flight.
    pub(crate) pending_compaction_id: Arc<Mutex<Option<ChatEntryId>>>,
}

impl CompactionWorker {
    /// Creates a new compaction worker.
    pub fn new(
        services: Services,
        handle: Handle,
        state: State,
        config: CompactionConfig,
        compaction_prompt: String,
    ) -> Self {
        Self {
            services,
            handle,
            state,
            config,
            compaction_prompt,
            compaction_in_progress: Arc::new(AtomicBool::new(false)),
            pending_compaction_id: Arc::new(Mutex::new(None)),
        }
    }

    /// Check whether the compaction summary from a previous auto-compaction
    /// has been applied by scanning the provided snapshot for its entry ID.
    ///
    /// If found, clears the in-progress flag and pending ID. Returns true
    /// if the flag was cleared (compaction applied), false if still in flight.
    fn check_compaction_applied(&self, history: &[ChatEntry]) -> bool {
        let guard = self.pending_compaction_id.lock().expect("lock");
        let Some(pending_id) = guard.as_ref() else {
            return false;
        };
        let found = history.iter().any(|entry| entry.id == *pending_id);
        drop(guard);

        if found {
            self.clear_compaction_state();
            tracing::info!("compaction summary found in snapshot, clearing in-progress flag");
        }

        found
    }

    /// Clear the compaction in-progress state (flag and pending ID).
    fn clear_compaction_state(&self) {
        self.compaction_in_progress.store(false, Ordering::SeqCst);
        *self.pending_compaction_id.lock().expect("lock") = None;
    }
}

#[async_trait::async_trait]
impl HistoryWorker for CompactionWorker {
    fn name(&self) -> &'static str {
        "compaction"
    }

    async fn evaluate(
        &self,
        session_id: &SessionId,
        history: Arc<[ChatEntry]>,
    ) -> Vec<HistoryMutation> {
        // Delegate to evaluate_history which needs state access.
        // The history snapshot actor provides the shared history snapshot.
        self.evaluate_history(session_id, &history).await
    }
}

impl CompactionWorker {
    /// Full evaluation with session state access.
    ///
    /// # Errors
    ///
    /// Returns an error if the LLM call fails or the response cannot be parsed.
    ///
    /// # Panics
    ///
    /// Panics if the preferences subsystem cannot be accessed.
    pub async fn evaluate_for_session(
        &self,
        trigger: &CompactionTrigger,
    ) -> Result<Vec<HistoryMutation>, error_stack::Report<CompactionError>> {
        // Load preferences from service (outside state lock).
        let prefs = self
            .services
            .user_preferences_storage
            .load()
            .expect("preferences");

        // Read session state.
        let (config, model_name, history, compaction_prompt, retry_config) = {
            let state = self.state.read();
            let session = state.session(&trigger.session_id);
            let config = prefs.compaction.clone();
            let model_name = session.profile().model.clone();
            let history = session.history().to_vec();
            let compaction_prompt = state.context.compaction_prompt.clone();
            let retry_config = prefs.request_retry.to_retry_config();
            (config, model_name, history, compaction_prompt, retry_config)
        };

        if history.is_empty() {
            return Ok(vec![]);
        }

        let result = self
            .evaluate_with_config(
                &history,
                &config,
                &model_name,
                &compaction_prompt,
                &retry_config,
                trigger.compact_all,
                ChatEntryId::new(),
            )
            .await;

        match result {
            Ok(mutations) => Ok(mutations),
            Err(e) => {
                tracing::error!(
                    session_id = %trigger.session_id,
                    error = %e,
                    "compaction worker evaluation failed"
                );
                Err(e)
            }
        }
    }

    /// Evaluate history using just the history snapshot (for HistoryWorker trait).
    ///
    /// This is the auto-compaction path (triggered by `HistoryAppended`).
    /// Compacts only when the session's tiktoken-based `context_size()` (the same
    /// value shown in the status bar) exceeds `config.threshold` of the model's
    /// `context_length`.
    async fn evaluate_history(
        &self,
        session_id: &SessionId,
        history: &[ChatEntry],
    ) -> Vec<HistoryMutation> {
        // --- Compaction deduplication gate ---
        // If a previous auto-compaction is in flight, check whether the
        // snapshot contains the compaction summary entry. If found, the
        // mutations were applied — clear the flag and proceed. If not found,
        // the LLM call or mutation application is still pending — skip.
        if self.compaction_in_progress.load(Ordering::SeqCst)
            && !self.check_compaction_applied(history)
        {
            tracing::info!(
                session_id = %session_id,
                "auto-compaction in flight, skipping snapshot"
            );
            return vec![];
        }

        // Load preferences from service (outside state lock).
        let prefs = self
            .services
            .user_preferences_storage
            .load()
            .expect("preferences");

        let (config, model_name, compaction_prompt, retry_config, full_history) = {
            let state = self.state.read();
            let config = prefs.compaction.clone();
            let Some(session) = state.session.get(session_id) else {
                return vec![];
            };
            let model_name = session.profile().model.clone();
            let compaction_prompt = state.context.compaction_prompt.clone();
            let retry_config = prefs.request_retry.to_retry_config();

            // --- Threshold gate ---
            // Uses the exact same values displayed in the status bar:
            //   - context_size() = tiktoken count from last prompt assembly
            //   - context_length = model's context window from provider/model cache
            // If either is unavailable, we skip compaction (can't determine threshold).
            let Some(context_size) = session.context_size() else {
                tracing::debug!(
                    session_id = %session_id,
                    "skipping compaction: context_size not yet calculated"
                );
                return vec![];
            };

            let context_length =
                resolve_context_limit(state.provider.model_cache.as_ref(), &model_name);

            let context_limit = match context_length {
                Some(limit) => limit,
                None => {
                    // No model context_length available. Use fallback from config.
                    config.fallback_context_window as u32
                }
            };

            let usage_ratio = f64::from(context_size) / f64::from(context_limit);
            if usage_ratio < config.threshold {
                tracing::debug!(
                    session_id = %session_id,
                    context_size,
                    context_limit,
                    usage_ratio,
                    threshold = config.threshold,
                    "skipping compaction: below threshold"
                );
                return vec![];
            }

            tracing::info!(
                session_id = %session_id,
                context_size,
                context_limit,
                usage_ratio,
                threshold = config.threshold,
                "context exceeds threshold, proceeding with compaction"
            );

            let history = session.history().to_vec();
            (config, model_name, compaction_prompt, retry_config, history)
        };

        // Pre-generate the compaction entry ID and mark as in-flight.
        let compaction_entry_id = ChatEntryId::new();
        self.compaction_in_progress.store(true, Ordering::SeqCst);
        *self.pending_compaction_id.lock().expect("lock") = Some(compaction_entry_id.clone());

        let result = self
            .evaluate_with_config(
                &full_history,
                &config,
                &model_name,
                &compaction_prompt,
                &retry_config,
                false, // Not compact_all for auto-trigger
                compaction_entry_id,
            )
            .await;

        match result {
            Ok(mutations) if mutations.is_empty() => {
                // Nothing to compact — clear flag so next snapshot can re-evaluate.
                self.clear_compaction_state();
                mutations
            }
            Ok(mutations) => mutations,
            Err(e) => {
                tracing::error!(error = %e, "compaction worker evaluation failed");
                self.clear_compaction_state();
                vec![]
            }
        }
    }

    /// Core evaluation algorithm.
    ///
    /// `pub(crate)` for testing - the real entry points are [`evaluate`] (trait)
    /// and [`evaluate_for_session`] (trigger-based).
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    pub(crate) async fn evaluate_with_config(
        &self,
        history: &[ChatEntry],
        config: &CompactionConfig,
        model_name: &str,
        compaction_prompt: &str,
        retry_config: &jinn_provider::RetryConfig,
        compact_all: bool,
        compaction_entry_id: ChatEntryId,
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
            // Nothing to compact - all tokens fit within the reserve.
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
            id: compaction_entry_id,

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

        let last_gathered_id = gathered_indices.last().map(|&idx| history[idx].id.clone());
        mutations.push(HistoryMutation::InsertEntry {
            after_entry_id: last_gathered_id,
            entry: compaction_entry,
        });

        Ok(mutations)
    }
}

/// Look up the context_length for the active model from the model cache.
///
/// Mirrors the same lookup used by the status bar display so the compaction
/// threshold gate and the status bar percentage are always consistent.
fn resolve_context_limit(
    model_cache: Option<&crate::feat::provider_infra::ModelCache>,
    active_model: &str,
) -> Option<u32> {
    model_cache.and_then(|cache| {
        let provider_name = active_model.split('/').next()?;
        let models = cache.entries.get(provider_name)?;
        let model_suffix = &active_model[(provider_name.len() + 1)..];
        models
            .iter()
            .find(|m| m.id == model_suffix)
            .and_then(|m| m.context_length)
    })
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
    retry_config: &jinn_provider::RetryConfig,
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
             Update the existing summary to reflect the new information.\n\n",
        );
    } else {
        user_content.push_str(
            "The messages below are a conversation to summarize. \
             Create a brief summary.\n\n",
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
            let estimator = CharRatioEstimator;
            let mut full_response = String::new();
            pin_mut!(stream);
            while let Some(result) = stream.next().await {
                match result {
                    Ok(token) => {
                        full_response.push_str(&token);
                        if estimator.estimate(&full_response) >= MAX_SUMMARY_TOKENS {
                            tracing::warn!(
                                tokens = estimator.estimate(&full_response),
                                "compaction summary hit token ceiling, truncating"
                            );
                            break;
                        }
                    }
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
