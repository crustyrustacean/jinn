//! Single-session and whole-tree archiving.

use std::collections::{HashMap, HashSet, VecDeque};

use jinn_core_types::SessionId;
use jinn_core_types::SessionProfile;
use jinn_domain::common::actor_deps::BusPublish;
use jinn_session_state::{ChatSessionState, SessionSnapshot, snapshot_frozen_node};
use jinn_session_store_msg::SessionState;
use jinn_session_store_msg::{ArchiveSession, ArchiveSessionTree};

use jinn_session_msg::{SessionArchived, SessionClosed, SessionRemoved, SessionSeed};

use crate::session_store_actor::SessionStoreActor;

impl SessionStoreActor {
    /// Archives a session without running a teardown script.
    pub(crate) async fn handle_archive_session(&self, payload: &ArchiveSession) {
        self.archive_members(std::slice::from_ref(&payload.session_id))
            .await;
    }

    /// Archives a session and all descendants, all-or-nothing.
    pub(crate) async fn handle_archive_session_tree(&self, payload: &ArchiveSessionTree) {
        let Some(members) = self.guarded_tree_closure(&payload.root).await else {
            return;
        };
        self.archive_members(&members).await;
    }

    /// Resolves the tree and aborts before any side effect when a member is busy.
    async fn guarded_tree_closure(&self, root: &SessionId) -> Option<Vec<SessionId>> {
        let members = self.resolve_tree_closure(root).await;
        let state = self.state.read();
        let busy = members.iter().any(|id| {
            state.session.get(id).is_some_and(|session| {
                session.is_busy() || !matches!(session.phase(), jinn_session_msg::PhaseKind::Idle)
            })
        });
        drop(state);
        if busy {
            tracing::warn!(root = %root, "tree action aborted: a member session is busy");
            return None;
        }
        Some(members)
    }

    /// Resolves a root's subtree across loaded sessions and store summaries.
    async fn resolve_tree_closure(&self, root: &SessionId) -> Vec<SessionId> {
        let mut parent_of = self.parent_links_from_memory();
        match self.services.session_store.load_summaries().await {
            Ok(summaries) => {
                for summary in summaries {
                    parent_of
                        .entry(summary.session_id)
                        .or_insert(summary.parent_session);
                }
            }
            Err(error) => {
                tracing::warn!(
                    root = %root,
                    ?error,
                    "could not read store for tree closure; using memory only"
                );
            }
        }
        build_closure(root, &parent_of)
    }

    /// Snapshots parent links for every loaded session.
    fn parent_links_from_memory(&self) -> HashMap<SessionId, Option<SessionId>> {
        self.state
            .read()
            .session
            .iter()
            .map(|(id, session)| (id.clone(), session.parent_session().clone()))
            .collect()
    }

    /// Archives all requested members durably before changing live state.
    async fn archive_members(&self, members: &[SessionId]) {
        let Some(snapshots) = self.archive_snapshots(members).await else {
            return;
        };
        if let Err(error) = self
            .services
            .session_store
            .archive_snapshots(&snapshots)
            .await
        {
            tracing::warn!(
                ?error,
                member_count = members.len(),
                "archive write failed; live sessions remain intact"
            );
            return;
        }

        for session_id in members {
            if !self.state.read().session.contains(session_id) {
                continue;
            }
            self.snapshot_before_removal(session_id);
            let (removed_parent, mcp_enablement) = self.remove_and_replace(session_id);
            self.publish(SessionRemoved {
                session_id: session_id.clone(),
                removed_parent,
            })
            .await;
            self.publish(SessionArchived {
                session_id: session_id.clone(),
            })
            .await;
            self.publish(SessionClosed {
                session_id: session_id.clone(),
            })
            .await;
            if let Some(enablement) = mcp_enablement {
                self.publish(enablement).await;
            }
        }
    }

    /// Captures one complete archived snapshot for each requested member.
    ///
    /// A member that is live uses the authoritative in-memory state. A member
    /// that is not live is loaded from the store so an archive tree can update
    /// persisted descendants without making them live first.
    async fn archive_snapshots(&self, members: &[SessionId]) -> Option<Vec<SessionSnapshot>> {
        let mut snapshots = Vec::with_capacity(members.len());
        for session_id in members {
            let mut snapshot = {
                let state = self.state.read();
                state
                    .session
                    .get(session_id)
                    .map(ChatSessionState::capture_snapshot)
            };
            if snapshot.is_none() {
                snapshot = match self.services.session_store.load_session(session_id).await {
                    Ok(Some(snapshot)) => Some(snapshot),
                    Ok(None) => continue,
                    Err(error) => {
                        tracing::warn!(
                            ?error,
                            session_id = %session_id,
                            "could not load member for archive; leaving live state intact"
                        );
                        return None;
                    }
                };
            }
            let Some(mut snapshot) = snapshot else {
                continue;
            };
            if snapshot.revision.get() == 0 {
                snapshot.revision = jinn_session_state::SessionRevision::new(1);
            }
            snapshot.metadata.session_state = SessionState::Archived;
            snapshots.push(snapshot);
        }
        (!snapshots.is_empty()).then_some(snapshots)
    }

    /// Captures immutable tree statistics before dropping the live session.
    fn snapshot_before_removal(&self, session_id: &SessionId) {
        let frozen = self
            .state
            .read()
            .session
            .get(session_id)
            .map(snapshot_frozen_node);
        if let Some(frozen) = frozen {
            self.state.with_session(|view| {
                view.session.insert_frozen_node(frozen);
            });
        }
    }

    /// Removes a session, creates a seeded replacement, and returns the
    /// removed session's persisted parent plus any replacement MCP notice.
    fn remove_and_replace(
        &self,
        session_id: &SessionId,
    ) -> (
        Option<SessionId>,
        Option<jinn_mcp_msg::McpEnablementChanged>,
    ) {
        let (fresh_session, enablement) = {
            let app_state = self.services.app_state_storage.read();
            let preferences = self.services.user_preferences_storage.read();
            let mut profile = SessionProfile::from_model_selection(
                app_state.last_model.clone().unwrap_or_default(),
            );
            profile.reasoning_effort = app_state.reasoning_effort;
            let seed = SessionSeed::from_preferences(&preferences);
            profile.disabled_tools.clone_from(&seed.disabled_tools);
            profile.disabled_skills.clone_from(&seed.disabled_skills);

            let mut fresh = ChatSessionState::new_with_profile(profile);
            fresh.set_enabled_mcp_servers(seed.enabled_mcp.clone());
            let enablement =
                seed.has_auto_enabled_mcp()
                    .then(|| jinn_mcp_msg::McpEnablementChanged {
                        session_id: fresh.session_id().clone(),
                        enabled: seed.enabled_mcp,
                    });
            (fresh, enablement)
        };

        let removed_parent = self
            .state
            .read()
            .session
            .get(session_id)
            .and_then(|session| session.parent_session().clone());
        self.state.with_session(|view| {
            view.session
                .map()
                .remove_and_replace(session_id, fresh_session);
        });
        (removed_parent, enablement)
    }
}

/// Builds a cycle-safe descendant closure in breadth-first order.
fn build_closure(
    root: &SessionId,
    parent_of: &HashMap<SessionId, Option<SessionId>>,
) -> Vec<SessionId> {
    let mut children_of: HashMap<SessionId, Vec<SessionId>> = HashMap::new();
    for (id, parent) in parent_of {
        if let Some(parent_id) = parent {
            children_of
                .entry(parent_id.clone())
                .or_default()
                .push(id.clone());
        }
    }

    let mut closure = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::from([root.clone()]);
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        closure.push(id.clone());
        if let Some(children) = children_of.get(&id) {
            queue.extend(children.iter().cloned());
        }
    }
    closure
}
