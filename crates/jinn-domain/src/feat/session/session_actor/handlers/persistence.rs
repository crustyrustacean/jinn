//! Persistence handlers - save and load session snapshots.

use std::collections::{HashMap, HashSet};

use super::super::SessionPersistenceActor;
use crate::SessionLoadRequested;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::feat::session::tree_aggregate::snapshot_frozen_node;
use crate::protocol::{Event, SessionId};

impl SessionPersistenceActor {
    /// Saves the current state of a session to disk.
    ///
    /// Clones the session inside `spawn_blocking` to avoid blocking the
    /// async runtime with a potentially expensive `ChatSessionState` clone
    /// (which includes the full `Vec<ChatEntry>` history). The store's
    /// `save` method does its own `spawn_blocking` internally for SQLite I/O.
    /// Errors are logged as warnings - persistence failure must not break
    /// the user experience.
    pub(in crate::feat::session::session_actor) async fn save_active_session(
        &self,
        session_id: &crate::protocol::SessionId,
    ) {
        let store = &self.services.session_store;

        let state = self.state.clone();
        let session_id = session_id.clone();
        let session_id_log = session_id.clone();

        let session = tokio::task::spawn_blocking(move || {
            let mut guard = state.write();
            let session = guard.session.get_mut(&session_id)?;
            session.touch();
            Some(session.clone())
        })
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(err = ?e, "spawn_blocking panicked during session save");
            None
        });

        let Some(session) = session else { return };

        // Guard: don't persist sessions the user hasn't interacted with.
        if !session.is_persistable() {
            return;
        }

        if let Err(e) = store.save(&session).await {
            tracing::warn!(
                session_id = ?session_id_log,
                err = ?e,
                "failed to persist session"
            );
        }
    }

    /// Marks a session as having been interacted with by the user.
    ///
    /// Sets `has_interacted = true` on the session and emits a `UserInteracted` event.
    pub(in crate::feat::session::session_actor) async fn handle_mark_session_interacted(
        &mut self,
        payload: &crate::feat::session::protocol::mark_session_interacted::MarkSessionInteracted,
        ctx: &crate::common::actor::ActorContext,
    ) {
        {
            let mut state = self.state.write();
            if let Some(session) = state.session.get_mut(&payload.session_id) {
                session.mark_interacted();
            }
        }

        if let Err(e) = ctx.send_event(crate::protocol::Event::UserInteracted(
            crate::feat::session::protocol::user_interacted::UserInteracted {
                session_id: payload.session_id.clone(),
            },
        )) {
            tracing::warn!(err = ?e, "session-actor failed to emit UserInteracted");
        }

        self.save_active_session(&payload.session_id).await;
    }

    /// Inserts a session into the session map and emits [`SessionLoadCompleted`].
    ///
    /// This is the single canonical "load a session" path. Every site that
    /// inserts a [`ChatSessionState`] into `state.session` must go through
    /// this method so that external subscribers (token-count actor, sidebar,
    /// etc.) are notified.
    ///
    /// # Guarantees
    ///
    /// The session is inserted **before** the event is emitted, so
    /// subscribers can look it up by ID immediately.
    pub(in crate::feat::session::session_actor) fn load_and_insert(
        &self,
        session: crate::feat::session::chat_session::ChatSessionState,
        ctx: &crate::common::actor::ActorContext,
    ) {
        use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted as CompletedPayload;

        let session_id = session.session_id().clone();
        {
            let mut state = self.state.write();
            state.session.insert(session.clone());
            // Remove the frozen node snapshot - the live session replaces it.
            state.session.remove_frozen_node(&session_id);
        }
        let _ = ctx.send_event(Event::SessionLoadCompleted(Box::new(CompletedPayload {
            session,
        })));
    }

    /// Creates an empty session with the given ID and emits a `SessionLoadCompleted` command.
    ///
    /// Used as a fallback when a session is not found or fails to load.
    #[expect(clippy::unused_self, reason = "trait contract requires #[allow(clippy::unused_self)]self method")]
    fn create_empty_session_response(
        &self,
        session_id: &crate::protocol::SessionId,
        ctx: &crate::common::actor::ActorContext,
    ) {
        use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted as CompletedPayload;

        let mut session = crate::feat::session::chat_session::ChatSessionState::new();
        session.set_session_id(session_id.clone());
        let _ = ctx.send_event(Event::SessionLoadCompleted(Box::new(CompletedPayload {
            session,
        })));
    }

    /// Hydrate frozen nodes for all live sessions' tree members.
    ///
    /// Called at startup after loading unarchived sessions. For each live session,
    /// walks the tree to find members not in memory and creates frozen node snapshots.
    pub(in crate::feat::session::session_actor) async fn hydrate_all_tree_frozen_nodes(
        &self,
        store: &crate::feat::session::SessionStoreService,
    ) {
        // Load all summaries to get tree structure.
        let summaries = match store.load_summaries().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(err = ?e, "failed to load summaries for tree hydration");
                return;
            }
        };

        // Build a lookup: session_id → (parent_session_id)
        let summary_map: HashMap<SessionId, Option<SessionId>> = summaries
            .iter()
            .map(|s| (s.session_id.clone(), s.parent_session.clone()))
            .collect();

        // Collect all tree IDs across all live sessions.
        let mut all_tree_ids = HashSet::new();
        {
            let state = self.state.read();
            for id in state.session.sessions().keys() {
                let tree_ids = Self::collect_tree_ids(id, &summary_map);
                all_tree_ids.extend(tree_ids);
            }
        }

        // For each tree member not already live or frozen, load full session
        // and create a frozen node.
        let mut frozen_to_insert = Vec::new();
        {
            let state = self.state.read();
            for id in &all_tree_ids {
                if state.session.contains(id) || state.session.frozen_nodes().contains_key(id) {
                    continue;
                }
                frozen_to_insert.push(id.clone());
            }
        }

        if frozen_to_insert.is_empty() {
            return;
        }

        tracing::info!(
            tree_members = all_tree_ids.len(),
            need_frozen = frozen_to_insert.len(),
            "hydrating frozen nodes at startup"
        );

        // Load full sessions and create frozen nodes.
        let mut new_frozen_nodes = Vec::new();
        for id in &frozen_to_insert {
            match store.load_session(id).await {
                Ok(Some(session)) => {
                    let frozen = snapshot_frozen_node(&session);
                    new_frozen_nodes.push(frozen);
                }
                Ok(None) => {
                    tracing::debug!(session_id = %id, "session in tree not found in store, skipping frozen node");
                }
                Err(e) => {
                    tracing::warn!(session_id = %id, err = ?e, "failed to load session for frozen node");
                }
            }
        }

        // Insert all new frozen nodes.
        if !new_frozen_nodes.is_empty() {
            let mut state = self.state.write();
            for node in new_frozen_nodes {
                state.session.insert_frozen_node(node);
            }
        }
    }

    /// Loads frozen node snapshots for all sessions in the loaded session's tree
    /// that are not already in memory.
    ///
    /// When a session is loaded from disk (e.g., via the sidebar picker), its
    /// ancestors and siblings may never have been in memory this app session.
    /// Without frozen nodes for them, `find_tree_root()` sees an orphan and
    /// the tree summary disappears.
    ///
    /// This method loads all session summaries, walks the tree to find all
    /// member session IDs, and for each member not already live or frozen,
    /// loads the full session, creates a frozen node snapshot, and inserts it.
    /// The full session is then discarded (not kept live).
    ///
    /// Runs once at session load time - the frozen nodes are cached in memory.
    pub(in crate::feat::session::session_actor) async fn hydrate_tree_frozen_nodes(
        &self,
        store: &crate::feat::session::SessionStoreService,
        loaded_session_id: &SessionId,
    ) {
        // Load all summaries to get tree structure.
        let summaries = match store.load_summaries().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(err = ?e, "failed to load summaries for tree hydration");
                return;
            }
        };

        // Build a lookup: session_id → (parent_session_id)
        let summary_map: HashMap<SessionId, Option<SessionId>> = summaries
            .iter()
            .map(|s| (s.session_id.clone(), s.parent_session.clone()))
            .collect();

        // Walk up to find root, then BFS to collect all tree members.
        let tree_ids = Self::collect_tree_ids(loaded_session_id, &summary_map);

        // For each tree member not already live or frozen, load full session
        // and create a frozen node.
        let mut frozen_to_insert = Vec::new();
        {
            let state = self.state.read();
            for id in &tree_ids {
                // Skip if already live or already frozen.
                if state.session.contains(id) || state.session.frozen_nodes().contains_key(id) {
                    continue;
                }
                frozen_to_insert.push(id.clone());
            }
        }

        if frozen_to_insert.is_empty() {
            return;
        }

        tracing::info!(
            loaded_session = %loaded_session_id,
            tree_size = tree_ids.len(),
            need_frozen = frozen_to_insert.len(),
            "hydrating frozen nodes for tree members"
        );

        // Load full sessions and create frozen nodes.
        let mut new_frozen_nodes = Vec::new();
        for id in &frozen_to_insert {
            match store.load_session(id).await {
                Ok(Some(session)) => {
                    let frozen = snapshot_frozen_node(&session);
                    new_frozen_nodes.push(frozen);
                }
                Ok(None) => {
                    tracing::debug!(session_id = %id, "session in tree not found in store, skipping frozen node");
                }
                Err(e) => {
                    tracing::warn!(session_id = %id, err = ?e, "failed to load session for frozen node");
                }
            }
        }

        // Insert all new frozen nodes.
        if !new_frozen_nodes.is_empty() {
            let mut state = self.state.write();
            for node in new_frozen_nodes {
                state.session.insert_frozen_node(node);
            }
        }
    }

    /// Walk the parent chain up to root, then BFS to collect all session IDs
    /// in the tree. Uses only the summary map (session_id → parent_session_id).
    fn collect_tree_ids(
        start: &SessionId,
        summary_map: &HashMap<SessionId, Option<SessionId>>,
    ) -> HashSet<SessionId> {
        // Walk up to find root.
        let mut visited = HashSet::new();
        let mut current = start.clone();
        loop {
            if !visited.insert(current.clone()) {
                break; // cycle
            }
            let Some(Some(parent)) = summary_map.get(&current) else {
                break; // no parent or not in summaries
            };
            if !summary_map.contains_key(parent) {
                break; // parent not in summaries
            }
            current = parent.clone();
        }
        let root = current;

        // BFS from root to collect all tree members.
        let mut tree = HashSet::new();
        let mut queue = vec![root];
        while let Some(id) = queue.pop() {
            if !tree.insert(id.clone()) {
                continue;
            }
            // Find all children of this node.
            for (child_id, parent_id) in summary_map {
                if parent_id.as_ref() == Some(&id) && !tree.contains(child_id) {
                    queue.push(child_id.clone());
                }
            }
        }

        tree
    }

    /// Loads a full session from disk and sends back a `SessionLoadCompleted` command.
    ///
    /// If the requested session is a judge, redirects to loading its origin session
    /// instead. The judge gets loaded as a side-effect of the origin's auto-load.
    pub(in crate::feat::session::session_actor) async fn on_load_requested(
        &mut self,
        evt: &SessionLoadRequested,
        ctx: &crate::common::actor::ActorContext,
    ) {
        let store = self.services.session_store.clone();

        match store.load_session(&evt.session_id).await {
            Ok(Some(mut session)) => {
                // Unarchive the session so it appears in the picker on next load.
                if let Err(e) = store.set_archived(&evt.session_id, false).await {
                    tracing::warn!(err = ?e, "failed to unarchive session on load");
                }

                // Reset in-memory state so the sidebar filter includes this session.
                session.set_session_state(crate::feat::session::chat_session::SessionState::Loaded);

                // Insert into state and emit SessionLoadCompleted for subscribers.
                self.load_and_insert(session, ctx);

                // Run the full restore flow (CWD validation, context size, persist).
                let session_id = evt.session_id.clone();
                let session = self
                    .state
                    .read()
                    .session
                    .get(&session_id)
                    .expect("just inserted")
                    .clone();
                let payload = SessionLoadCompleted { session };
                self.handle_session_load_completed(&payload, ctx).await;

                // Hydrate frozen nodes for tree members not in memory.
                // This ensures the tree summary shows ancestors/siblings even
                // when they were never loaded into memory this app session.
                self.hydrate_tree_frozen_nodes(&store, &evt.session_id)
                    .await;
            }
            Ok(None) => {
                tracing::warn!(
                    session_id = ?evt.session_id,
                    "session load returned None"
                );
                self.create_empty_session_response(&evt.session_id, ctx);
            }
            Err(e) => {
                tracing::warn!(err = ?e, "failed to load session");
                self.create_empty_session_response(&evt.session_id, ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::similar_names,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::super::super::helpers::{test_actor_with_store, test_context};
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::feat::session::chat_session::SessionState;
    use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;
    use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;
    use crate::protocol::Event;

    #[tokio::test]
    async fn loading_archived_session_resets_state_to_loaded() {
        // Given an archived session in the store.
        let mut store_session = ChatSessionState::new();
        store_session.set_title("Archived Chat".to_owned());
        store_session.set_session_state(SessionState::Archived);
        let session_id = store_session.session_id().clone();
        let (mut actor, _store) = test_actor_with_store(vec![store_session]);
        let (sink, ctx) = test_context();

        // When loading the archived session.
        actor
            .on_load_requested(
                &SessionLoadRequested {
                    session_id: session_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then SessionLoadCompleted is emitted with session_state == Loaded.
        let loaded_session = sink
            .events()
            .iter()
            .find_map(|cmd| match cmd {
                Event::SessionLoadCompleted(payload) => Some(payload.session.clone()),
                _ => None,
            })
            .expect("expected SessionLoadCompleted command");

        assert_eq!(
            loaded_session.session_state(),
            SessionState::Loaded,
            "loaded session should have SessionState::Loaded"
        );

        // And the session appears in sorted_open_sessions.
        let mut state = actor.state.write();
        state
            .session
            .sessions_mut()
            .insert(session_id.clone(), loaded_session);
        state.session.set_active(session_id.clone());

        let sidebar_sessions = sorted_open_sessions(&state);
        assert!(
            sidebar_sessions.iter().any(|s| s.id == session_id),
            "archived session should appear in sidebar after loading"
        );
    }

    #[tokio::test]
    async fn save_active_session_skips_non_persistable_session() {
        // Given an actor with a new (non-interacted) session and a recording store.
        let (actor, store) = test_actor_with_store(vec![]);
        let session_id = actor.state.read().session.active_session_id().clone();

        // When saving the active session.
        actor.save_active_session(&session_id).await;

        // Then the session was NOT saved because it is not persistable.
        assert!(
            store.last_saved_session(&session_id).is_none(),
            "non-interacted session should not be persisted"
        );
    }

    #[tokio::test]
    async fn save_active_session_persists_interacted_session() {
        // Given an actor with an interacted session and a recording store.
        let (actor, store) = test_actor_with_store(vec![]);
        let session_id = actor.state.read().session.active_session_id().clone();
        {
            let mut state = actor.state.write();
            state.active_session_mut().mark_interacted();
        }

        // When saving the active session.
        actor.save_active_session(&session_id).await;

        // Then the session was saved.
        assert!(
            store.last_saved_session(&session_id).is_some(),
            "interacted session should be persisted"
        );
    }

    #[tokio::test]
    async fn handle_mark_session_interacted_sets_flag_emits_event_and_persists() {
        use crate::feat::session::protocol::mark_session_interacted::MarkSessionInteracted;
        use crate::protocol::Event;

        // Given an actor with a new session.
        let (mut actor, store) = test_actor_with_store(vec![]);
        let session_id = actor.state.read().session.active_session_id().clone();
        let (sink, ctx) = test_context();

        // When handling MarkSessionInteracted.
        actor
            .handle_mark_session_interacted(
                &MarkSessionInteracted {
                    session_id: session_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then the session has_interacted flag is set.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(session.has_interacted());
        assert!(session.is_persistable());

        // And a UserInteracted event was emitted.
        let has_event = sink
            .events()
            .iter()
            .any(|e| matches!(e, Event::UserInteracted(e) if e.session_id == session_id));
        assert!(has_event, "UserInteracted event should be emitted");

        // And the session was persisted to the store.
        assert!(
            store.last_saved_session(&session_id).is_some(),
            "interacted session should be persisted after MarkSessionInteracted"
        );
    }

    #[tokio::test]
    async fn loading_child_session_creates_frozen_node_for_archived_parent() {
        // Given a parent and a child session in the store.
        let mut parent = ChatSessionState::new();
        parent.set_title("Parent Session".to_owned());
        parent.mark_interacted();
        parent.push_entry(crate::protocol::ChatEntry::user("parent msg"));
        let parent_id = parent.session_id().clone();

        let mut child = ChatSessionState::new();
        child.set_title("Child Session".to_owned());
        child.mark_interacted();
        child.set_parent_session(parent_id.clone());
        child.push_entry(crate::protocol::ChatEntry::user("child msg"));
        let child_id = child.session_id().clone();

        let (mut actor, _store) = test_actor_with_store(vec![parent, child]);
        let (_sink, ctx) = test_context();

        // When loading the child session.
        actor
            .on_load_requested(
                &SessionLoadRequested {
                    session_id: child_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then the parent has a frozen node (not loaded as a live session).
        let state = actor.state.read();
        let frozen = state.session.frozen_nodes();
        assert!(
            frozen.contains_key(&parent_id),
            "parent should have a frozen node after child is loaded"
        );

        // And the child is live.
        assert!(
            state.session.contains(&child_id),
            "child should be in live sessions"
        );
    }
}
