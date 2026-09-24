//! Single-session and whole-tree archiving.

use std::collections::{HashMap, HashSet, VecDeque};

use jinn_core_types::SessionId;
use jinn_domain::common::actor_deps::BusPublish;
use jinn_domain::feat::session::chat_session::{ChatSessionState, SessionState};
use jinn_domain::feat::session::profile::{SessionProfile, SessionSeed};
use jinn_domain::feat::session::protocol::archive_session::ArchiveSession;
use jinn_domain::feat::session::protocol::archive_session_tree::ArchiveSessionTree;
use jinn_domain::feat::session::protocol::session_archived::SessionArchived;
use jinn_domain::feat::session::protocol::session_closed::SessionClosed;
use jinn_domain::feat::session::sessions_list::reconcile::reconcile_split;
use jinn_domain::feat::session::sessions_list::state::update_visual_parents_on_removal_split;
use jinn_domain::feat::session::snapshot_frozen_node;

use crate::session_store_actor::SessionStoreActor;

impl SessionStoreActor {
    /// Archives a session without running a teardown script.
    pub(crate) async fn handle_archive_session(&self, payload: &ArchiveSession) {
        self.archive_live_member(&payload.session_id).await;
    }

    /// Archives a session and all descendants, all-or-nothing.
    pub(crate) async fn handle_archive_session_tree(&self, payload: &ArchiveSessionTree) {
        let Some(members) = self.guarded_tree_closure(&payload.root).await else {
            return;
        };

        for member_id in &members {
            if !self.state.read().session.contains(member_id) {
                continue;
            }
            self.archive_live_member(member_id).await;
        }

        if let Err(error) = self
            .services
            .session_store
            .set_archived_many(&members, true)
            .await
        {
            tracing::warn!(
                root = %payload.root,
                ?error,
                "archive tree store writeback failed (memory state is consistent)"
            );
        }
    }

    /// Resolves the tree and aborts before any side effect when a member is busy.
    async fn guarded_tree_closure(&self, root: &SessionId) -> Option<Vec<SessionId>> {
        let members = self.resolve_tree_closure(root).await;
        let state = self.state.read();
        let busy = members.iter().any(|id| {
            state.session.get(id).is_some_and(|session| {
                session.is_busy()
                    || !matches!(
                        session.phase(),
                        jinn_domain::feat::session::phase_machine::PhaseKind::Idle
                    )
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

    /// Archives and removes one loaded member, publishing its close events.
    async fn archive_live_member(&self, session_id: &SessionId) {
        {
            self.state.with_session(&self.session_cap, |view| {
                if let Some(session) = view.session.map().get_mut(session_id) {
                    session.set_session_state(SessionState::Archived);
                }
            });
        }
        self.save_active_session(session_id).await;
        self.snapshot_before_removal(session_id);
        let mcp_enablement = self.remove_and_replace(session_id);

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

    /// Captures immutable tree statistics before dropping the live session.
    fn snapshot_before_removal(&self, session_id: &SessionId) {
        let frozen = self
            .state
            .read()
            .session
            .get(session_id)
            .map(snapshot_frozen_node);
        if let Some(frozen) = frozen {
            self.state.with_session(&self.session_cap, |view| {
                view.session.insert_frozen_node(frozen);
            });
        }
    }

    /// Removes a session, creates a seeded replacement, and reconciles the UI.
    ///
    /// Returns the replacement's MCP-enablement notification for publication
    /// after the state lock is released.
    fn remove_and_replace(
        &self,
        session_id: &SessionId,
    ) -> Option<jinn_mcp_msg::McpEnablementChanged> {
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

        self.state
            .with_session_sidebar(&self.session_cap, &self.frontend_cap, |view| {
                update_visual_parents_on_removal_split(
                    view.session.map(),
                    view.frontend,
                    session_id,
                );
                view.session
                    .map()
                    .remove_and_replace(session_id, fresh_session);
                reconcile_split(view.session.map(), view.frontend);
            });
        enablement
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
