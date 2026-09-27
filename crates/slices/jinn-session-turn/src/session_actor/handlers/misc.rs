//! Miscellaneous handlers - skills refresh display and history mutation intake.

use super::super::SessionPersistenceActor;
use jinn_context_assembly_msg::ContextOverrideChanged;
use jinn_kernel::common::actor_deps::BusPublish;
use jinn_session_history_msg::SubmitHistoryMutations;
use jinn_session_msg::PhaseKind;
use jinn_skills_msg::{Skill, SkillsLoaded};

use jinn_kernel::protocol::ChatEntry;
use jinn_preferences_config::schemas::AutoPruneConfig;

impl SessionPersistenceActor {
    /// Pushes a transient entry listing discovered skills.
    pub(in crate::session_actor) fn on_skills_loaded(&self, event: &SkillsLoaded) {
        // Only show a message when the skill picker is active (manual refresh).
        // Startup scans arrive while no picker is open.
        //
        // The picker is slice-owned, so its open state is a question about the
        // focus scope rather than a picker kind: the skills slice pushes its
        // own dynamic scope, and asking "is that scope on top?" keeps this
        // handler free of any picker identity.
        let is_picker_active = {
            let state = self.state.read();
            matches!(
                state.frontend.scope(),
                jinn_kernel::FocusScope::Dynamic(ref scope) if scope == &jinn_skills_msg::skill_picker_scope()
            )
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

        self.state.with_session(|view| {
            if let Some(session) = view.session.map().get_mut(&event.session_id) {
                session.push_entry(ChatEntry::transient(content));
            }
        });
    }

    /// Queues a batch of history mutations for deferred application.
    ///
    /// Workers submit `Vec<HistoryMutation>` batches via the
    /// `SubmitHistoryMutations` command. This handler pushes them to
    /// `pending_mutations` and applies them immediately if the session is
    /// idle (no active stream). If the session is streaming or sending,
    /// mutations are deferred until the next stream completion.
    pub(in crate::session_actor) async fn handle_submit_history_mutations(
        &self,
        payload: &SubmitHistoryMutations,
    ) {
        if payload.mutations.is_empty() {
            return;
        }

        // Resolve token costs for all incoming context-override mutations in a
        // single read-guard pass, before taking the write lock. The cost map is
        // then consumed by the accumulator inside the write lock without any
        // self-borrow (which would deadlock against the held write guard).
        let threshold = self
            .services
            .config
            .get::<AutoPruneConfig>()
            .unwrap_or_default()
            .accumulation_threshold_tokens;
        let token_costs: std::collections::HashMap<jinn_core_types::ChatEntryId, u32> = {
            use jinn_llm_support::token_estimator::TokenCounter;
            let state = self.state.read();
            let session = state.session.get(&payload.session_id);
            payload
                .mutations
                .iter()
                .filter_map(|m| match m {
                    jinn_core_types::HistoryMutation::SetContextOverride {
                        entry_id,
                        source,
                        ..
                    } => {
                        // Only prune ForcedExclude mutations reach the
                        // accumulator, so only their cost is relevant.
                        // Worker ForcedInclude and compaction overrides
                        // apply immediately and need no cost.
                        if is_compaction_source(source) {
                            None
                        } else {
                            Some(entry_id)
                        }
                    }
                    _ => None,
                })
                .map(|entry_id| {
                    let cost = self
                        .token_cache
                        .get(&payload.session_id, entry_id)
                        .or_else(|| {
                            session
                                .and_then(|s| s.history().iter().find(|e| &e.id == entry_id))
                                .and_then(|e| e.prompt_text())
                                .map(|t| self.counter.count(t) as u32)
                        })
                        .unwrap_or(0);
                    (entry_id.clone(), cost)
                })
                .collect()
        };

        // Capture what changed (if anything) so events can be emitted after releasing the write lock.
        let (session_id, changed) = {
            self.state.with_session(|view| {
                let session = view.session.map().get_or_create(&payload.session_id);

                for mutation in payload.mutations.clone() {
                    if is_prune_override(&mutation) {
                        // Pruner ForcedExclude: route into the accumulation buffer
                        // so it counts toward the batch flush threshold.
                        if let jinn_core_types::HistoryMutation::SetContextOverride {
                            entry_id,
                            value,
                            source,
                        } = &mutation
                        {
                            let cost = token_costs.get(entry_id).copied().unwrap_or(0);
                            session.route_override(entry_id.clone(), *value, source.clone(), cost);
                        }
                    } else {
                        // All other mutations apply immediately:
                        //   - compaction overrides (compaction is itself a context reduction),
                        //   - worker ForcedInclude (protection, never a prune),
                        //   - any non-context mutation.
                        // Only prune ForcedExclude is subject to the accumulation gate.
                        session.queue_mutations(vec![mutation]);
                    }
                }

                // Flush the accumulator if its deduplicated token total crossed the threshold.
                session.flush_accumulated_overrides_if_needed(threshold);

                tracing::debug!(
                    session_id = %payload.session_id,
                    queue_len = session.pending_mutation_count(),
                    accumulated = session.accumulated_overrides_total(),
                    threshold,
                    "routed history mutations from worker"
                );

                // If the session is idle (no active stream), drain immediately.
                // Otherwise mutations wait for the next stream completion.
                if matches!(session.phase(), PhaseKind::Idle) {
                    let (_count, changed) = session.drain_and_apply_pending_mutations();
                    (payload.session_id.clone(), changed)
                } else {
                    (payload.session_id.clone(), Vec::new())
                }
            })
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

/// Whether a mutation is a prune `ForcedExclude` override — the only
/// direction subject to the accumulation gate.
///
/// The accumulator exists to batch pruner excludes into a single flush so
/// the server-side KV cache isn't invalidated per-entry. Worker
/// `ForcedInclude` (protection), compaction overrides, and any non-context
/// mutation apply immediately instead.
fn is_prune_override(mutation: &jinn_core_types::HistoryMutation) -> bool {
    use jinn_core_types::HistoryMutation;
    matches!(
        mutation,
        HistoryMutation::SetContextOverride {
            value: jinn_core_types::ContextOverride::ForcedExclude,
            source,
            ..
        } if !is_compaction_source(source)
    )
}

/// Whether a `ChangeSource` is the compaction worker.
///
/// Compaction overrides are exempt from the accumulation gate: compaction is
/// itself a context reduction that must apply promptly, and holding back its
/// excludes would leave the gathered entries and the new summary both in
/// context simultaneously.
fn is_compaction_source(source: &jinn_kernel::protocol::ChangeSource) -> bool {
    matches!(source, jinn_kernel::protocol::ChangeSource::Worker { name } if name == "compaction")
}
/// Builds a markdown message listing discovered skills.
fn build_skills_refresh_message(skills: &[Skill]) -> String {
    let mut msg = format!("Skills refreshed: {} found\n\n", skills.len());
    for skill in skills {
        msg.push_str("- ");
        msg.push_str(&skill.name);
        msg.push('\n');
    }
    msg
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        clippy::unnecessary_mut_passed,
        reason = "test code"
    )]
    use crate::session_actor::helpers::test_actor_recording;
    use jinn_core_types::SessionId;
    use jinn_kernel::protocol::{ChangeSource, ChatEntry};

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_submit_history_mutations_buffers_subthreshold_override_when_idle() {
        // Given a default (10_000) accumulation threshold and one user entry.
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            state.session.active_session_id().clone()
        };
        let entry_id = {
            let state = actor.state.read();
            state.session.get(&session_id).unwrap().history()[0]
                .id
                .clone()
        };

        // When submitting a single sub-threshold ForcedExclude override.
        actor
            .handle_submit_history_mutations(&jinn_session_history_msg::SubmitHistoryMutations {
                session_id: session_id.clone(),
                mutations: vec![jinn_core_types::HistoryMutation::SetContextOverride {
                    entry_id: entry_id.clone(),
                    value: jinn_core_types::ContextOverride::ForcedExclude,
                    source: ChangeSource::Internal {
                        label: "test".to_owned(),
                    },
                }],
            })
            .await;

        // Then the override is buffered (not applied) and no pending batch exists.
        let state = actor.state.read();
        let session = state.session.get(&session_id).unwrap();
        assert_eq!(
            session.history()[0].context_override(),
            jinn_core_types::ContextOverride::Default
        );
        assert!(!session.has_pending_mutations());
        assert!(
            session.accumulated_prune_count() > 0,
            "sub-threshold override should be buffered in the accumulator"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_submit_history_mutations_with_empty_batch_is_noop() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        actor
            .handle_submit_history_mutations(&jinn_session_history_msg::SubmitHistoryMutations {
                session_id: session_id.clone(),
                mutations: vec![],
            })
            .await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).unwrap();
        assert!(!session.has_pending_mutations());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_submit_history_mutations_creates_session_if_missing() {
        let (actor, _audit) = test_actor_recording().await;
        let new_session_id = SessionId::new();

        actor
            .handle_submit_history_mutations(&jinn_session_history_msg::SubmitHistoryMutations {
                session_id: new_session_id.clone(),
                mutations: vec![jinn_core_types::HistoryMutation::SetContextOverride {
                    entry_id: jinn_core_types::ChatEntryId::new(),
                    value: jinn_core_types::ContextOverride::ForcedExclude,
                    source: ChangeSource::Internal {
                        label: "test".to_owned(),
                    },
                }],
            })
            .await;

        let state = actor.state.read();
        let session = state.session.get(&new_session_id).unwrap();
        assert!(!session.has_pending_mutations());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_submit_history_mutations_buffers_prune_override_and_applies_include_immediately()
     {
        // Given a default (10_000) accumulation threshold and two user entries.
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("first"));
            session.push_entry(ChatEntry::user("second"));
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

        // When submitting a sub-threshold ForcedExclude (prune) for entry 1
        // and a ForcedInclude (worker protection) for entry 2.
        actor
            .handle_submit_history_mutations(&jinn_session_history_msg::SubmitHistoryMutations {
                session_id: session_id.clone(),
                mutations: vec![jinn_core_types::HistoryMutation::SetContextOverride {
                    entry_id: entry_id_1,
                    value: jinn_core_types::ContextOverride::ForcedExclude,
                    source: ChangeSource::Internal {
                        label: "test".to_owned(),
                    },
                }],
            })
            .await;
        actor
            .handle_submit_history_mutations(&jinn_session_history_msg::SubmitHistoryMutations {
                session_id: session_id.clone(),
                mutations: vec![jinn_core_types::HistoryMutation::SetContextOverride {
                    entry_id: entry_id_2,
                    value: jinn_core_types::ContextOverride::ForcedInclude,
                    source: ChangeSource::Internal {
                        label: "test".to_owned(),
                    },
                }],
            })
            .await;

        // Then the ForcedExclude is buffered (entry 1 still Default) and the
        // ForcedInclude applies immediately (entry 2 is ForcedInclude), so only
        // one override sits in the accumulator.
        let state = actor.state.read();
        let session = state.session.get(&session_id).unwrap();
        assert_eq!(session.pending_mutation_count(), 0);
        assert_eq!(
            session.history()[0].context_override(),
            jinn_core_types::ContextOverride::Default
        );
        assert_eq!(
            session.history()[1].context_override(),
            jinn_core_types::ContextOverride::ForcedInclude
        );
        assert_eq!(
            session.accumulated_prune_count(),
            1,
            "only the ForcedExclude (prune) should be buffered; the include applies immediately"
        );
    }
    #[rstest::rstest]
    #[tokio::test]
    async fn handle_submit_history_mutations_emits_context_override_changed_on_change() {
        let (actor, audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            state.session.active_session_id().clone()
        };
        let entry_id = {
            let state = actor.state.read();
            state.session.get(&session_id).unwrap().history()[0]
                .id
                .clone()
        };

        actor
            .handle_submit_history_mutations(&jinn_session_history_msg::SubmitHistoryMutations {
                session_id: session_id.clone(),
                mutations: vec![jinn_core_types::HistoryMutation::SetContextOverride {
                    entry_id: entry_id.clone(),
                    value: jinn_core_types::ContextOverride::ForcedExclude,
                    source: ChangeSource::Worker {
                        name: "compaction".to_owned(),
                    },
                }],
            })
            .await;

        assert!(
            audit.contains_name("ContextOverrideChanged"),
            "expected ContextOverrideChanged to be emitted for worker-applied change"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_submit_history_mutations_does_not_emit_on_noop_mutation() {
        let (actor, audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            let id = session.session_id().clone();
            session.set_entry_context_override_at(
                0,
                jinn_core_types::ContextOverride::ForcedExclude,
                &ChangeSource::Internal {
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

        actor
            .handle_submit_history_mutations(&jinn_session_history_msg::SubmitHistoryMutations {
                session_id: session_id.clone(),
                mutations: vec![jinn_core_types::HistoryMutation::SetContextOverride {
                    entry_id: entry_id.clone(),
                    value: jinn_core_types::ContextOverride::ForcedExclude,
                    source: ChangeSource::Worker {
                        name: "test_worker".to_owned(),
                    },
                }],
            })
            .await;

        assert!(
            !audit.contains_name("ContextOverrideChanged"),
            "expected no ContextOverrideChanged for no-op mutation"
        );
        let state = actor.state.read();
        let session = state.session.get(&session_id).unwrap();
        assert_eq!(
            session.history()[0].context_history.len(),
            1,
            "context_history should contain only the setup event"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn worker_forced_include_override_applies_immediately_not_buffered() {
        // Given a session with one assistant entry and the default 10_000 threshold.
        let (actor, _audit) = test_actor_recording().await;
        let (session_id, entry_id) = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            let entry = ChatEntry::assistant("response");
            let id = entry.id.clone();
            session.push_entry(entry);
            (state.session.active_session_id().clone(), id)
        };

        // When submitting a worker ForcedInclude override (protection, never a prune).
        actor
            .handle_submit_history_mutations(&jinn_session_history_msg::SubmitHistoryMutations {
                session_id: session_id.clone(),
                mutations: vec![jinn_core_types::HistoryMutation::SetContextOverride {
                    entry_id: entry_id.clone(),
                    value: jinn_core_types::ContextOverride::ForcedInclude,
                    source: ChangeSource::Worker {
                        name: "auto-prune-todo".to_owned(),
                    },
                }],
            })
            .await;

        // Then the override applied immediately (no buffering) because worker
        // includes never count toward the prune threshold.
        let state = actor.state.read();
        let session = state.session.get(&session_id).unwrap();
        assert_eq!(
            session.history()[0].context_override(),
            jinn_core_types::ContextOverride::ForcedInclude,
            "worker ForcedInclude must apply immediately"
        );
        assert!(
            session.accumulated_prune_count() == 0,
            "worker ForcedInclude must not enter the accumulation buffer"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn compaction_forced_exclude_override_applies_immediately_not_buffered() {
        // Given a session with one assistant entry.
        let (actor, _audit) = test_actor_recording().await;
        let (session_id, entry_id) = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            let entry = ChatEntry::assistant("response");
            let id = entry.id.clone();
            session.push_entry(entry);
            (state.session.active_session_id().clone(), id)
        };

        // When submitting a compaction ForcedExclude override.
        actor
            .handle_submit_history_mutations(&jinn_session_history_msg::SubmitHistoryMutations {
                session_id: session_id.clone(),
                mutations: vec![jinn_core_types::HistoryMutation::SetContextOverride {
                    entry_id: entry_id.clone(),
                    value: jinn_core_types::ContextOverride::ForcedExclude,
                    source: ChangeSource::Worker {
                        name: "compaction".to_owned(),
                    },
                }],
            })
            .await;

        // Then the override applied immediately and did not enter the buffer.
        let state = actor.state.read();
        let session = state.session.get(&session_id).unwrap();
        assert_eq!(
            session.history()[0].context_override(),
            jinn_core_types::ContextOverride::ForcedExclude,
            "compaction ForcedExclude must apply immediately"
        );
        assert!(
            session.accumulated_prune_count() == 0,
            "compaction ForcedExclude must not enter the accumulation buffer"
        );
    }
}
