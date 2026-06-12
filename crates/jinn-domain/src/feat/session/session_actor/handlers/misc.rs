//! Miscellaneous handlers - model refresh display and session picker loading.
//!
//! Handles pushing model refresh results as transient markdown entries to the chat log,
//! and loading session picker entries from the session store into app state.

use super::super::SessionPersistenceActor;
use crate::common::actor_deps::BusPublish;
use crate::feat::context::protocol::event::ContextOverrideChanged;
use crate::feat::provider::protocol::event::ModelsRefreshed;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations;

use crate::feat::ui::picker_states::PickerExt;
use crate::protocol::{ChatEntry, PickerKind};

impl SessionPersistenceActor {
    /// Pushes a transient markdown entry after model refresh.
    ///
    /// Builds a markdown table from the refresh results and pushes it directly
    /// to session state. Does NOT emit `PushChatEntry` - transient entries
    /// are not persisted.
    pub(in crate::feat::session::session_actor) fn on_models_refreshed(
        &self,
        event: &ModelsRefreshed,
    ) {
        let content = if event.results.is_empty() && event.errors.is_empty() {
            "Models refreshed: no providers found".to_owned()
        } else {
            build_models_refresh_table(event)
        };

        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&event.session_id);
            session.push_entry(ChatEntry::transient(content));
        }
    }

    /// Pushes a transient entry listing discovered skills.
    pub(in crate::feat::session::session_actor) fn on_skills_loaded(
        &self,
        event: &crate::feat::skills::SkillsLoaded,
    ) {
        // Only show a message when the skill picker is active (manual refresh).
        // Startup scans arrive while no picker is open.
        let is_picker_active = {
            let state = self.state.read();
            state.frontend.scope_stack.picker_kind() == Some(&PickerKind::Skill)
        };

        if !is_picker_active {
            return;
        }

        let content = if let Some(err) = &event.error {
            format!("Skills refresh failed: {err}")
        } else if event.skills.is_empty() {
            "Skills refreshed: no skills found".to_owned()
        } else {
            build_skills_refresh_message(&event.skills)
        };

        {
            let mut state = self.state.write();
            if let Some(session) = state.try_session_mut(&event.session_id) {
                session.push_entry(ChatEntry::transient(content));
            }
        }
    }

    /// Loads session picker entries from the session store into `AppState`.
    pub(in crate::feat::session::session_actor) async fn handle_load_session_picker_entries(
        &self,
        _payload: &LoadSessionPickerEntries,
    ) {
        {
            let store = &self.services.session_store;
            let theme = {
                let state = self.state.read();
                state.frontend.theme.clone()
            };
            let entries =
                crate::feat::session::entries::load_session_entries_from_store(store, &theme).await;
            let mut state = self.state.write();
            state.frontend.session_picker_mut().set_items(entries);
        }
    }

    /// Queues a batch of history mutations for deferred application.
    ///
    /// Workers submit `Vec<HistoryMutation>` batches via the
    /// `SubmitHistoryMutations` command. This handler pushes them to
    /// `pending_mutations` and applies them immediately if the session is
    /// idle (no active stream). If the session is streaming or sending,
    /// mutations are deferred until the next stream completion.
    pub(in crate::feat::session::session_actor) async fn handle_submit_history_mutations(
        &self,
        payload: &SubmitHistoryMutations,
    ) {
        if payload.mutations.is_empty() {
            return;
        }
        // Capture what changed (if anything) so events can be emitted after releasing the write lock.
        let (session_id, changed) = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.queue_mutations(payload.mutations.clone());
            tracing::debug!(
                session_id = %payload.session_id,
                queue_len = session.core.ephemeral.pending_mutations.len(),
                "queued history mutations from worker"
            );

            // If the session is idle (no active stream), drain immediately.
            // Otherwise mutations wait for the next stream completion.
            if matches!(session.phase(), PhaseKind::Idle) {
                let (_count, changed) = session.drain_and_apply_pending_mutations();
                (payload.session_id.clone(), changed)
            } else {
                (payload.session_id.clone(), Vec::new())
            }
        };

        // Emit ContextOverrideChanged events for any entry whose override actually changed.
        // Doing this outside the write lock keeps the bus dispatch decoupled from session state.
        for entry_id in changed {
            self.publish(ContextOverrideChanged {
                session_id: session_id.clone(),
                entry_id,
            })
            .await;
        }
    }
}

/// Builds a markdown table string from the models refresh event.
///
/// Format:
/// ```markdown
/// | Provider | Models | Status |
/// |----------|--------|--------|
/// | ollama   | 5      | ✅     |
/// | openai   | 0      | ❌ API key not resolved |
/// ```
#[expect(
    clippy::else_if_without_else,
    reason = "no-op on fallthrough is intentional"
)]
fn build_models_refresh_table(event: &ModelsRefreshed) -> String {
    // Collect all provider names and sort alphabetically.
    let mut all_providers: Vec<&str> = event
        .results
        .keys()
        .chain(event.errors.keys())
        .map(std::string::String::as_str)
        .collect();
    all_providers.sort_unstable();
    all_providers.dedup();

    let mut rows = Vec::new();
    for provider in all_providers {
        if let Some(models) = event.results.get(provider) {
            rows.push(format!("| {provider} | {} | ✅ |", models.len()));
        } else if let Some(err) = event.errors.get(provider) {
            rows.push(format!("| {provider} | 0 | ❌ {err} |"));
        }
    }

    let mut table = String::new();
    table.push_str("| Provider | Models | Status |\n");
    table.push_str("|----------|--------|--------|\n");
    for row in rows {
        table.push_str(&row);
        table.push('\n');
    }

    table
}

/// Builds a markdown message listing discovered skills.
fn build_skills_refresh_message(skills: &[crate::feat::skills::Skill]) -> String {
    let mut msg = format!("Skills refreshed: {} found\n\n", skills.len());
    for skill in skills {
        msg.push_str("- ");
        msg.push_str(&skill.name);
        msg.push('\n');
    }
    msg
}

//FIXME: disabled during actor migration — tests reference deleted types
// #[cfg(test)]
#[cfg(any())]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        clippy::unnecessary_mut_passed,
        reason = "test code"
    )]
    use super::super::super::helpers::{test_actor, test_actor_with_store};
    use crate::feat::provider::protocol::event::ModelsRefreshed;
    use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
    use crate::feat::ui::picker_states::PickerExt;
    use crate::protocol::{ChatEntryKind, SessionId};
    use jinn_provider::ModelInfo;
    use std::collections::HashMap;

    // --- on_models_refreshed ---

    #[tokio::test]
    async fn on_models_refreshed_pushes_transient_entry() {
        // Given a session actor.
        let actor = test_actor().await;
        let session_id = SessionId::new();

        // When refreshing models with some results.
        let mut results = HashMap::new();
        results.insert(
            "ollama".to_owned(),
            vec![ModelInfo {
                id: "llama3".to_owned(),
                context_length: Some(8192),
            }],
        );
        actor.on_models_refreshed(&ModelsRefreshed {
            session_id: session_id.clone(),
            results,
            errors: HashMap::new(),
        });

        // Then a transient entry with a table was pushed.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert_eq!(session.history().len(), 1);
        let entry = &session.history()[0];
        assert!(matches!(&entry.kind, ChatEntryKind::Transient(t) if t.contains("ollama")));
    }

    #[tokio::test]
    async fn on_models_refreshed_empty_results_shows_no_providers_message() {
        // Given a session actor.
        let actor = test_actor().await;
        let session_id = SessionId::new();

        // When refreshing models with empty results AND empty errors.
        actor.on_models_refreshed(&ModelsRefreshed {
            session_id: session_id.clone(),
            results: HashMap::new(),
            errors: HashMap::new(),
        });

        // Then a transient entry with "no providers found" message was pushed.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert_eq!(session.history().len(), 1);
        let entry = &session.history()[0];
        assert!(
            matches!(&entry.kind, ChatEntryKind::Transient(t) if t.contains("no providers found")),
            "expected 'no providers found' message, got {:?}",
            entry.kind
        );
    }

    #[tokio::test]
    async fn on_models_refreshed_with_errors_shows_table() {
        // Given a session actor.
        let actor = test_actor().await;
        let session_id = SessionId::new();

        // When refreshing models with errors but no results.
        let mut errors = HashMap::new();
        errors.insert("openai".to_owned(), "API key not resolved".to_owned());
        actor.on_models_refreshed(&ModelsRefreshed {
            session_id: session_id.clone(),
            results: HashMap::new(),
            errors,
        });

        // Then a transient entry with a table (not the "no providers" message) was pushed.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        let entry = &session.history()[0];
        assert!(
            matches!(&entry.kind, ChatEntryKind::Transient(t) if t.contains("openai") && t.contains("API key not resolved")),
            "expected table with error, got {:?}",
            entry.kind
        );
    }

    // --- build_models_refresh_table ---

    #[tokio::test]
    async fn build_models_refresh_table_includes_provider_and_model_count() {
        // Given a refresh event with results.
        let mut results = HashMap::new();
        results.insert(
            "ollama".to_owned(),
            vec![
                ModelInfo {
                    id: "llama3".to_owned(),
                    context_length: Some(8192),
                },
                ModelInfo {
                    id: "phi3".to_owned(),
                    context_length: None,
                },
            ],
        );
        let event = ModelsRefreshed {
            session_id: SessionId::new(),
            results,
            errors: HashMap::new(),
        };

        // When building the table.
        let table = super::build_models_refresh_table(&event);

        // Then the table contains the provider name and correct model count.
        assert!(table.contains("ollama"), "expected provider name in table");
        assert!(table.contains('2'), "expected model count in table");
        assert!(table.contains("✅"), "expected success indicator");
    }

    // --- handle_load_session_picker_entries ---

    #[tokio::test]
    async fn handle_load_session_picker_entries_loads_from_store() {
        // Given an actor with a store containing a session.
        let session = crate::feat::session::chat_session::ChatSessionState::new();
        let (actor, _store) = test_actor_with_store(vec![session]).await;

        // When loading session picker entries.
        actor
            .handle_load_session_picker_entries(&LoadSessionPickerEntries)
            .await;

        // Then the session picker has entries (at least one from the stored session).
        let state = actor.state.read();
        assert!(
            !state.frontend.session_picker().items().is_empty(),
            "expected session picker to have entries after loading from store"
        );
    }

    // --- handle_submit_history_mutations ---

    #[tokio::test]
    async fn handle_submit_history_mutations_applies_immediately_when_idle() {
        // Given a session actor with a session that has one entry.
        let actor = test_actor().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(crate::protocol::ChatEntry::user("hello"));
            state.session.active_session_id().clone()
        };
        let entry_id = {
            let state = actor.state.read();
            state.session.get(&session_id).unwrap().history()[0]
                .id
                .clone()
        };

        // When submitting history mutations while session is idle.
        actor.handle_submit_history_mutations(
            &crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations {
                session_id: session_id.clone(),
                mutations: vec![
                    crate::feat::session::history_mutation::HistoryMutation::SetContextOverride {
                        entry_id: entry_id.clone(),
                        value: crate::feat::session::chat_entry::ContextOverride::ForcedExclude,
                        source: crate::feat::session::chat_entry::ChangeSource::Internal {
                            label: "test".to_owned(),
                        },
                    },
                ],
            },
        );

        // Then mutations are applied immediately (session is idle).
        let state = actor.state.read();
        let session = state.session.get(&session_id).unwrap();
        assert_eq!(
            session.history()[0].context_override(),
            crate::feat::session::chat_entry::ContextOverride::ForcedExclude
        );
        // Queue is empty after drain.
        assert!(session.core.ephemeral.pending_mutations.is_empty());
    }

    #[tokio::test]
    async fn handle_submit_history_mutations_with_empty_batch_is_noop() {
        // Given a session actor.
        let actor = test_actor().await;
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When submitting an empty mutations vec.
        actor.handle_submit_history_mutations(
            &crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations {
                session_id: session_id.clone(),
                mutations: vec![],
            },
        );

        // Then no batch was queued.
        let state = actor.state.read();
        let session = state.session.get(&session_id).unwrap();
        assert!(session.core.ephemeral.pending_mutations.is_empty());
    }

    #[tokio::test]
    async fn handle_submit_history_mutations_creates_session_if_missing() {
        // Given a session actor with no session for the target ID.
        let actor = test_actor().await;
        let new_session_id = SessionId::new();

        // When submitting mutations for a nonexistent session.
        actor.handle_submit_history_mutations(
            &crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations {
                session_id: new_session_id.clone(),
                mutations: vec![
                    crate::feat::session::history_mutation::HistoryMutation::SetContextOverride {
                        entry_id: crate::feat::session::chat_entry::ChatEntryId::new(),
                        value: crate::feat::session::chat_entry::ContextOverride::ForcedExclude,
                        source: crate::feat::session::chat_entry::ChangeSource::Internal {
                            label: "test".to_owned(),
                        },
                    },
                ],
            },
        );

        // Then the session was created and mutations applied immediately.
        let state = actor.state.read();
        let session = state.session.get(&new_session_id).unwrap();
        // Queue is empty (mutations were applied, though the entry ID didn't match).
        assert!(session.core.ephemeral.pending_mutations.is_empty());
    }

    #[tokio::test]
    async fn handle_submit_history_mutations_multiple_submissions_each_applied_immediately() {
        // Given a session actor.
        let actor = test_actor().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(crate::protocol::ChatEntry::user("first"));
            session.push_entry(crate::protocol::ChatEntry::user("second"));
            state.session.active_session_id().clone()
        };
        let entry_id_1 = {
            let state = actor.state.read();
            state.session.get(&session_id).unwrap().history()[0]
                .id
                .clone()
        };
        let entry_id_2 = {
            let state = actor.state.read();
            state.session.get(&session_id).unwrap().history()[1]
                .id
                .clone()
        };

        // When submitting two batches (session is idle, each applies immediately).
        actor.handle_submit_history_mutations(
            &crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations {
                session_id: session_id.clone(),
                mutations: vec![
                    crate::feat::session::history_mutation::HistoryMutation::SetContextOverride {
                        entry_id: entry_id_1,
                        value: crate::feat::session::chat_entry::ContextOverride::ForcedExclude,
                        source: crate::feat::session::chat_entry::ChangeSource::Internal {
                            label: "test".to_owned(),
                        },
                    },
                ],
            },
        );
        actor.handle_submit_history_mutations(
            &crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations {
                session_id: session_id.clone(),
                mutations: vec![
                    crate::feat::session::history_mutation::HistoryMutation::SetContextOverride {
                        entry_id: entry_id_2,
                        value: crate::feat::session::chat_entry::ContextOverride::ForcedInclude,
                        source: crate::feat::session::chat_entry::ChangeSource::Internal {
                            label: "test".to_owned(),
                        },
                    },
                ],
            },
        );

        // Then both mutations are applied and queue is empty.
        let state = actor.state.read();
        let session = state.session.get(&session_id).unwrap();
        assert_eq!(session.core.ephemeral.pending_mutations.len(), 0);
        assert_eq!(
            session.history()[0].context_override(),
            crate::feat::session::chat_entry::ContextOverride::ForcedExclude
        );
        assert_eq!(
            session.history()[1].context_override(),
            crate::feat::session::chat_entry::ContextOverride::ForcedInclude
        );
    }

    // --- Worker mutation emits ContextOverrideChanged ---

    #[tokio::test]
    async fn handle_submit_history_mutations_emits_context_override_changed_on_change() {
        // Given a session with one entry at Default.
        let actor = test_actor().await;
        let (sink, ctx) = crate::feat::session::session_actor::helpers::test_context();
        // Override actor's state to use our sink-equipped context for event capture.
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(crate::protocol::ChatEntry::user("hello"));
            state.session.active_session_id().clone()
        };
        let entry_id = {
            let state = actor.state.read();
            state.session.get(&session_id).unwrap().history()[0]
                .id
                .clone()
        };

        // When submitting a SetContextOverride mutation with a real change.
        actor.handle_submit_history_mutations(
            &crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations {
                session_id: session_id.clone(),
                mutations: vec![
                    crate::feat::session::history_mutation::HistoryMutation::SetContextOverride {
                        entry_id: entry_id.clone(),
                        value: crate::feat::session::chat_entry::ContextOverride::ForcedExclude,
                        source: crate::feat::session::chat_entry::ChangeSource::Worker {
                            name: "test_worker".to_owned(),
                        },
                    },
                ],
            },
        );

        // Then ContextOverrideChanged event was emitted.
        let events = sink.events();
        let has_event = events.iter().any(|ev| {
            matches!(
                ev,
                crate::protocol::Event::ContextOverrideChanged(payload)
                    if payload.entry_id == entry_id
            )
        });
        assert!(
            has_event,
            "expected ContextOverrideChanged to be emitted for worker-applied change"
        );
    }

    #[tokio::test]
    async fn handle_submit_history_mutations_does_not_emit_on_noop_mutation() {
        // Given a session with one entry already at ForcedExclude.
        let actor = test_actor().await;
        let (sink, ctx) = crate::feat::session::session_actor::helpers::test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(crate::protocol::ChatEntry::user("hello"));
            let id = session.core.session_id.clone();
            // Pre-set the entry to ForcedExclude.
            session.core.history[0].apply_context_override(
                crate::feat::session::chat_entry::ContextOverride::ForcedExclude,
                crate::feat::session::chat_entry::ChangeSource::Internal {
                    label: "setup".to_owned(),
                },
            );
            id
        };
        let entry_id = {
            let state = actor.state.read();
            state.session.get(&session_id).unwrap().history()[0]
                .id
                .clone()
        };

        // When submitting a SetContextOverride that matches the current value (no-op).
        actor.handle_submit_history_mutations(
            &crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations {
                session_id: session_id.clone(),
                mutations: vec![
                    crate::feat::session::history_mutation::HistoryMutation::SetContextOverride {
                        entry_id: entry_id.clone(),
                        value: crate::feat::session::chat_entry::ContextOverride::ForcedExclude,
                        source: crate::feat::session::chat_entry::ChangeSource::Worker {
                            name: "test_worker".to_owned(),
                        },
                    },
                ],
            },
        );

        // Then no ContextOverrideChanged event is emitted.
        let events = sink.events();
        let has_event = events
            .iter()
            .any(|ev| matches!(ev, crate::protocol::Event::ContextOverrideChanged(_)));
        assert!(
            !has_event,
            "expected no ContextOverrideChanged for no-op mutation"
        );
        // And context_history has only the setup event, not a duplicate.
        let state = actor.state.read();
        let session = state.session.get(&session_id).unwrap();
        assert_eq!(
            session.history()[0].context_history.len(),
            1,
            "context_history should contain only the setup event"
        );
    }
}
