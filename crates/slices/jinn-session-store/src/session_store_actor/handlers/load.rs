//! Session loading, restoration, tree hydration, and forking.

use std::collections::{HashMap, HashSet};

use jinn_chat_log_view_msg::{ArmLayoutDeadline, DEFAULT_MIN_COLLAPSE_COUNT, LayoutChatSession};
use jinn_core_types::{ChatEntry, SessionId};
use jinn_domain::common::actor_deps::BusPublish;
use jinn_domain::feat::session::SessionStoreService;
use jinn_domain::feat::ui::chat_log::layout_supervisor::{LAYOUT_DEADLINE, LAYOUT_SUPERVISOR_PATH};
use jinn_domain::protocol::system::ActiveSessionChanged;
use jinn_session_state::{ChatSessionState, SessionSnapshot, snapshot_frozen_node_from_snapshot};
use jinn_session_store_msg::SessionForkRequested;
use jinn_session_store_msg::{
    ChatLogMeasureRequested, SessionLoadCompleted, SessionLoadRequested, SessionState,
};
use trouper::actor::ActorPath;
use trouper::context::MsgCtx;
use trouper::envelope::Address;

/// Default lines before a tool call or result is truncated, matching the
/// chat log renderer's own fallback.
const DEFAULT_TOOL_ENTRY_MAX_LINES: u16 = 6;

use crate::session_store_actor::SessionStoreActor;

impl SessionStoreActor {
    /// Inserts a loaded session and returns its ID.
    pub(crate) fn insert_loaded_session(&self, session: ChatSessionState) -> SessionId {
        let session_id = session.session_id().clone();
        self.state.with_session(|view| {
            view.session.map().insert(session);
            view.session.map().remove_frozen_node(&session_id);
        });
        session_id
    }

    /// Completes initialization of an explicitly loaded session, then publishes its ID.
    ///
    /// The load guard is deliberately *not* cleared here. The chat log's
    /// loading indication is driven by that guard, and clearing it the moment
    /// the session is in memory is what made the next frame run the whole
    /// layout pass — the freeze this hand-off exists to avoid. Instead the
    /// chat log is handed to the layout workers, and the completion actor
    /// clears the guard once the line counts are measured.
    pub(crate) async fn restore_loaded_session(
        &self,
        ctx: &mut MsgCtx<'_>,
        snapshot: SessionSnapshot,
    ) {
        let session_id = snapshot.metadata.session_id.clone();
        let model = if snapshot.metadata.profile.model.is_no_provider() {
            self.state
                .read()
                .frontend
                .app_state
                .last_model
                .clone()
                .unwrap_or_default()
        } else {
            snapshot.metadata.profile.model.clone()
        };
        let mut session = snapshot.restore_live();
        session.set_model(model);
        session.mark_interacted();
        // The snapshot may have been taken while the session was archived. Loading it
        // makes it live again, and the sidebar lists only `Loaded` sessions.
        session.set_session_state(SessionState::Loaded);
        let original_cwd = session.cwd().to_path_buf();

        // Everything the measurement needs, taken while the session is still
        // owned here. Once it is moved into the map only a borrow of its
        // history is reachable, and a worker thread cannot hold that — so the
        // deep clone has to happen now, or not at all.
        let layout_inputs = self.collect_layout_inputs(&session, &session_id);

        self.state.with_preferences(|ops| {
            ops.frontend().update_sections(|sections| {
                sections
                    .sessions
                    .visual_parents
                    .retain(|_id, parent| parent != &session_id);
            });
        });
        self.state.with_session(|view| {
            let map = view.session.map();
            map.remove_frozen_node(&session_id);
            map.insert(session);
            map.set_active(session_id.clone());
        });

        // Dispatched only now that the session is active, so the completion
        // actor's active-session check sees it and the workers measure the
        // session that is actually on screen.
        Self::dispatch_layout(ctx, &session_id, layout_inputs);

        let cwd_exists = tokio::fs::try_exists(&original_cwd).await.unwrap_or(false);
        if !cwd_exists {
            self.restore_missing_cwd(&session_id, &original_cwd);
        }

        self.publish(ActiveSessionChanged {
            session_id: session_id.clone(),
        })
        .await;
        self.save_active_session(&session_id).await;
        self.publish(SessionLoadCompleted { session_id }).await;
    }

    /// Measures an in-memory session's chat log, without reading it again.
    ///
    /// A session switched to from the sidebar is already hydrated, so it needs
    /// only the measurement — not the disk read a [`SessionLoadRequested`]
    /// would perform. The layout hand-off is otherwise identical to a freshly
    /// loaded session's, so it goes through the same dispatch.
    pub(crate) fn on_measure_requested(
        &self,
        ctx: &mut MsgCtx<'_>,
        payload: &ChatLogMeasureRequested,
    ) {
        let session_id = payload.session_id.clone();
        // The session is expected to be in the map: the frontend only asks for
        // a measurement of a session it is about to show, and every live
        // session is hydrated at startup. If it is not, the load guard this
        // measurement was meant to clear would stay up forever, so it is
        // cleared here rather than left for a worker that will never run.
        let Some(session) = self.state.read().session.get(&session_id).cloned() else {
            tracing::warn!(
                session_id = %session_id,
                "measure requested for a session that is not in memory"
            );
            self.clear_load();
            return;
        };

        // The requester's width, not one re-derived from state: the frontend
        // has already switched to this session, so its own width is the
        // never-rendered zero. Measuring there would publish counts no frame
        // can use and have the completion actor discard them as stale.
        let layout_inputs =
            self.collect_layout_inputs_at(&session, &session_id, payload.content_width);
        self.state.with_session(|view| {
            view.session.map().set_active(session_id.clone());
        });
        Self::dispatch_layout(ctx, &session_id, layout_inputs);
    }

    /// Hands a session's chat log to the layout workers and arms its deadline.
    ///
    /// The deadline is what stops a pathological job from stranding the user on
    /// a spinner: when it expires the supervisor clears the load guard and the
    /// next frame measures inline instead.
    fn dispatch_layout(
        ctx: &mut MsgCtx<'_>,
        session_id: &SessionId,
        layout_inputs: LayoutChatSession,
    ) {
        // Typed sends: the history travels as a live value and is never
        // serialized, and the route table hands the job to one worker of the
        // pool.
        ctx.send_to_any(LayoutChatSession {
            session_id: session_id.clone(),
            ..layout_inputs
        });
        ctx.send(
            Address::Path(ActorPath::new(LAYOUT_SUPERVISOR_PATH)),
            ArmLayoutDeadline {
                session_id: session_id.clone(),
                after: LAYOUT_DEADLINE,
            },
            None,
        );
    }

    /// Everything the layout workers need to measure a freshly loaded session.
    ///
    /// The content width is the one the chat log last rendered at, which is the
    /// width the next frame will use. Measuring at a guess instead would yield
    /// counts that first frame could not use, throwing the whole measurement
    /// away and leaving the frame to do exactly the work this hand-off exists
    /// to avoid.
    fn collect_layout_inputs(
        &self,
        session: &ChatSessionState,
        session_id: &SessionId,
    ) -> LayoutChatSession {
        // From the session that was on screen before this load, which is the
        // frame that will render the new one.
        let content_width = {
            let state = self.state.read();
            state
                .session
                .get(state.session.active_session_id())
                .map_or(0, ChatSessionState::content_width)
        };
        self.collect_layout_inputs_at(session, session_id, content_width)
    }

    /// The same inputs, at a width the caller has already resolved.
    ///
    /// Split out so a caller that knows the width — because it read it before
    /// changing the active session, and can no longer read it after — does not
    /// have to re-derive it from state that has since moved on.
    fn collect_layout_inputs_at(
        &self,
        session: &ChatSessionState,
        session_id: &SessionId,
        content_width: u16,
    ) -> LayoutChatSession {
        // One read guard for both reads: taking a second would deadlock.
        let state = self.state.read();
        let preferences = &state.frontend.preferences;
        LayoutChatSession {
            session_id: session_id.clone(),
            content_width,
            entries: session.history().to_vec(),
            // Read from the incoming session rather than the active one: the
            // session was not active when it was still owned here, and its own
            // view state is the one that will be measured.
            shown_ignored_blocks: session.shown_ignored_blocks_snapshot(),
            min_collapse_count: preferences
                .min_collapse_count
                .unwrap_or(DEFAULT_MIN_COLLAPSE_COUNT),
            tool_entry_max_lines: preferences
                .tool_entry_max_lines
                .unwrap_or(DEFAULT_TOOL_ENTRY_MAX_LINES),
        }
    }

    /// Replaces a missing working directory with the application default.
    fn restore_missing_cwd(&self, session_id: &SessionId, original_cwd: &std::path::Path) {
        let default_cwd = self.state.read().session.default_cwd().clone();
        self.state.with_session(|view| {
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
    pub(crate) async fn on_load_requested(
        &self,
        ctx: &mut MsgCtx<'_>,
        payload: &SessionLoadRequested,
    ) {
        let store = self.services.session_store.clone();
        match store.load_session(&payload.session_id).await {
            Ok(Some(session)) => {
                if let Err(error) = store.set_archived(&payload.session_id, false).await {
                    tracing::warn!(?error, "failed to unarchive session on load");
                }
                self.restore_loaded_session(ctx, session).await;
                self.hydrate_tree_frozen_nodes(&store, &payload.session_id)
                    .await;
            }
            Ok(None) => {
                tracing::warn!(
                    session_id = ?payload.session_id,
                    "session load returned None"
                );
                self.clear_load();
            }
            Err(error) => {
                tracing::warn!(?error, "failed to load session");
                self.clear_load();
            }
        }
    }

    /// Persists the source, forks it in the store, then restores the child.
    pub(crate) async fn on_session_fork_requested(
        &self,
        ctx: &mut MsgCtx<'_>,
        payload: &SessionForkRequested,
    ) {
        self.state.with_session(|view| {
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
                self.restore_loaded_session(ctx, session).await;
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
            .with_session(|view| view.session.map().clear_load());
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
        self.state.with_session(|view| {
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
    ) -> Vec<jinn_session_store_msg::FrozenTreeNode> {
        let mut nodes = Vec::new();
        for id in session_ids {
            match store.load_session(id).await {
                Ok(Some(snapshot)) => nodes.push(snapshot_frozen_node_from_snapshot(&snapshot)),
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
