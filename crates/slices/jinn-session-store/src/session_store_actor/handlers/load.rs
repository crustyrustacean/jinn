//! Session loading, restoration, tree hydration, and forking.

use std::collections::{HashMap, HashSet};

use jinn_core_types::SessionId;
use jinn_domain::common::actor_deps::BusPublish;
use jinn_domain::feat::session::SessionStoreService;
use jinn_domain::feat::session::chat_session::{ChatSessionState, SessionState};
use jinn_domain::feat::session::protocol::session_fork_requested::SessionForkRequested;
use jinn_domain::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use jinn_domain::feat::session::protocol::session_load_requested::SessionLoadRequested;
use jinn_domain::feat::session::snapshot_frozen_node;
use jinn_domain::protocol::{ChatEntry, system::ActiveSessionChanged};

use crate::session_store_actor::SessionStoreActor;

impl SessionStoreActor {
    /// Inserts a loaded session before publishing its completion event.
    pub(crate) async fn load_and_insert(&self, session: ChatSessionState) {
        let session_id = session.session_id().clone();
        self.state.with_session(&self.session_cap, |view| {
            view.session.map().insert(session.clone());
            view.session.map().remove_frozen_node(&session_id);
        });
        self.publish(SessionLoadCompleted { session }).await;
    }

    /// Restores a loaded session into active state and persists the result.
    pub(crate) async fn handle_session_load_completed(&self, payload: &SessionLoadCompleted) {
        let session_id = payload.session.session_id().clone();
        let original_cwd = {
            let model = if payload.session.model_selection().is_no_provider() {
                self.state
                    .read()
                    .frontend
                    .app_state
                    .last_model
                    .clone()
                    .unwrap_or_default()
            } else {
                payload.session.model_selection().clone()
            };

            self.state.with_session(&self.session_cap, |view| {
                view.session.map().insert(payload.session.clone());
            });
            self.state.with_preferences(&self.frontend_cap, |ops| {
                ops.frontend().update_sections(|sections| {
                    sections
                        .sessions
                        .visual_parents
                        .retain(|_id, parent| parent != &session_id);
                });
            });

            self.state.with_session(&self.session_cap, |view| {
                let map = view.session.map();
                let Some(session) = map.get_mut(&session_id) else {
                    return std::path::PathBuf::new();
                };
                session.set_model(model);
                session.mark_interacted();
                let cwd = session.cwd().to_path_buf();
                map.set_active(session_id.clone());
                map.clear_load();
                cwd
            })
        };

        let cwd_exists = tokio::fs::try_exists(&original_cwd).await.unwrap_or(false);
        if !cwd_exists {
            self.restore_missing_cwd(&session_id, &original_cwd);
        }

        self.publish(ActiveSessionChanged {
            session_id: session_id.clone(),
        })
        .await;
        self.save_active_session(&session_id).await;
    }

    /// Replaces a missing working directory with the application default.
    fn restore_missing_cwd(&self, session_id: &SessionId, original_cwd: &std::path::Path) {
        let default_cwd = self.state.read().session.default_cwd().clone();
        self.state.with_session(&self.session_cap, |view| {
            let Some(session) = view.session.map().get_mut(session_id) else {
                return;
            };
            session.push_entry(ChatEntry::system(format!(
                "Warning: working directory '{}' not found, falling back to '{}'",
                original_cwd.display(),
                default_cwd.display()
            )));
            session.set_cwd(default_cwd);
        });
    }

    /// Loads a full session from storage and restores it into active state.
    pub(crate) async fn on_load_requested(&self, payload: &SessionLoadRequested) {
        let store = self.services.session_store.clone();
        match store.load_session(&payload.session_id).await {
            Ok(Some(mut session)) => {
                if let Err(error) = store.set_archived(&payload.session_id, false).await {
                    tracing::warn!(?error, "failed to unarchive session on load");
                }
                session.set_session_state(SessionState::Loaded);
                self.load_and_insert(session).await;
                self.restore_loaded_session(&payload.session_id).await;
                self.hydrate_tree_frozen_nodes(&store, &payload.session_id)
                    .await;
            }
            Ok(None) => {
                tracing::warn!(
                    session_id = ?payload.session_id,
                    "session load returned None"
                );
                self.publish_empty_session(&payload.session_id).await;
            }
            Err(error) => {
                tracing::warn!(?error, "failed to load session");
                self.publish_empty_session(&payload.session_id).await;
            }
        }
    }

    /// Runs the user-facing restore flow for a session already in state.
    async fn restore_loaded_session(&self, session_id: &SessionId) {
        let Some(session) = self.state.read().session.get(session_id).cloned() else {
            return;
        };
        self.handle_session_load_completed(&SessionLoadCompleted { session })
            .await;
    }

    /// Publishes an empty fallback session when a requested load fails.
    async fn publish_empty_session(&self, session_id: &SessionId) {
        let mut session = ChatSessionState::new();
        session.set_session_id(session_id.clone());
        self.publish(SessionLoadCompleted { session }).await;
    }

    /// Persists the source, forks it in the store, then restores the child.
    pub(crate) async fn on_session_fork_requested(&self, payload: &SessionForkRequested) {
        self.state.with_session(&self.session_cap, |view| {
            if let Some(session) = view.session.map().get_mut(&payload.source_session_id) {
                session.mark_interacted();
            }
        });
        self.save_active_session(&payload.source_session_id).await;

        let new_id = match self
            .services
            .session_store
            .fork(&payload.source_session_id, payload.at_ordinal)
            .await
        {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(?error, "failed to fork session");
                self.clear_load();
                return;
            }
        };

        match self.services.session_store.load_session(&new_id).await {
            Ok(Some(session)) => {
                self.load_and_insert(session).await;
                self.restore_loaded_session(&new_id).await;
            }
            Ok(None) => {
                tracing::warn!("forked session not found after creation");
                self.clear_load();
            }
            Err(error) => {
                tracing::warn!(?error, "failed to load forked session");
                self.clear_load();
            }
        }
    }

    /// Clears the global session loading guard.
    fn clear_load(&self) {
        self.state
            .with_session(&self.session_cap, |view| view.session.map().clear_load());
    }

    /// Hydrates frozen nodes for the loaded session's whole tree.
    pub(crate) async fn hydrate_tree_frozen_nodes(
        &self,
        store: &SessionStoreService,
        loaded_session_id: &SessionId,
    ) {
        let Some(summary_map) = self.summary_parent_map(store).await else {
            return;
        };
        let tree_ids = collect_tree_ids(loaded_session_id, &summary_map);
        let missing = self.missing_tree_members(&tree_ids);
        self.load_frozen_members(store, missing, Some(loaded_session_id))
            .await;
    }

    /// Hydrates frozen nodes for every live session's tree at startup.
    pub(crate) async fn hydrate_all_tree_frozen_nodes(&self, store: &SessionStoreService) {
        let Some(summary_map) = self.summary_parent_map(store).await else {
            return;
        };
        let all_tree_ids = self
            .state
            .read()
            .session
            .iter()
            .flat_map(|(id, _)| collect_tree_ids(id, &summary_map))
            .collect::<HashSet<_>>();
        let missing = self.missing_tree_members(&all_tree_ids);
        self.load_frozen_members(store, missing, None).await;
    }

    /// Loads the parent-link map used to resolve session trees.
    async fn summary_parent_map(
        &self,
        store: &SessionStoreService,
    ) -> Option<HashMap<SessionId, Option<SessionId>>> {
        match store.load_summaries().await {
            Ok(summaries) => Some(
                summaries
                    .into_iter()
                    .map(|summary| (summary.session_id, summary.parent_session))
                    .collect(),
            ),
            Err(error) => {
                tracing::warn!(?error, "failed to load summaries for tree hydration");
                None
            }
        }
    }

    /// Selects tree members that are neither live nor already frozen.
    fn missing_tree_members(&self, tree_ids: &HashSet<SessionId>) -> Vec<SessionId> {
        let state = self.state.read();
        tree_ids
            .iter()
            .filter(|id| {
                !state.session.contains(id) && !state.session.frozen_nodes().contains_key(*id)
            })
            .cloned()
            .collect()
    }

    /// Loads missing full sessions and inserts their frozen snapshots.
    async fn load_frozen_members(
        &self,
        store: &SessionStoreService,
        missing: Vec<SessionId>,
        loaded_session_id: Option<&SessionId>,
    ) {
        if missing.is_empty() {
            return;
        }
        tracing::info!(
            loaded_session = loaded_session_id.map(ToString::to_string).as_deref(),
            need_frozen = missing.len(),
            "hydrating frozen nodes for tree members"
        );
        let frozen = self.load_frozen_nodes(store, &missing).await;
        if frozen.is_empty() {
            return;
        }
        self.state.with_session(&self.session_cap, |view| {
            for node in frozen {
                view.session.insert_frozen_node(node);
            }
        });
    }

    /// Loads each member that still exists and converts it to a tree snapshot.
    async fn load_frozen_nodes(
        &self,
        store: &SessionStoreService,
        session_ids: &[SessionId],
    ) -> Vec<jinn_domain::feat::session::FrozenTreeNode> {
        let mut nodes = Vec::new();
        for id in session_ids {
            match store.load_session(id).await {
                Ok(Some(session)) => nodes.push(snapshot_frozen_node(&session)),
                Ok(None) => {
                    tracing::debug!(session_id = %id, "session in tree not found in store, skipping frozen node");
                }
                Err(error) => {
                    tracing::warn!(session_id = %id, ?error, "failed to load session for frozen node");
                }
            }
        }
        nodes
    }
}

/// Walks to the root and returns the complete tree in breadth-first order.
fn collect_tree_ids(
    start: &SessionId,
    summary_map: &HashMap<SessionId, Option<SessionId>>,
) -> HashSet<SessionId> {
    let mut visited = HashSet::new();
    let mut current = start.clone();
    loop {
        if !visited.insert(current.clone()) {
            break;
        }
        let Some(Some(parent)) = summary_map.get(&current) else {
            break;
        };
        if !summary_map.contains_key(parent) {
            break;
        }
        current = parent.clone();
    }

    let mut tree = HashSet::new();
    let mut queue = vec![current];
    while let Some(id) = queue.pop() {
        if !tree.insert(id.clone()) {
            continue;
        }
        let children = summary_map
            .iter()
            .filter(|(child_id, parent_id)| {
                parent_id.as_ref() == Some(&id) && !tree.contains(*child_id)
            })
            .map(|(child_id, _)| child_id.clone());
        queue.extend(children);
    }
    tree
}
