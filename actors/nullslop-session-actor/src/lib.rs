//! Session persistence actor — writes session snapshots to disk.
//!
//! Subscribes to [`SessionSaveRequested`] events from the bus, constructs a
//! [`PersistedSession`], and writes it to the [`SessionStore`]. Runs
//! asynchronously on a tokio task — file I/O never blocks the sync bus loop.

use jiff::Timestamp;
use nullslop_actor::{Actor, ActorContext, ActorEnvelope, SystemMessage};
use nullslop_protocol::session::SessionLoadRequested;
use nullslop_protocol::{Event, PromptStrategyId, SessionSaveRequested};
use nullslop_session::{PersistedSession, SessionStoreService};

/// Direct message type (unused — the actor only responds to bus events).
pub enum SessionPersistenceDirectMsg {}

/// Persists session data to disk on `SessionSaveRequested` events.
///
/// Receives [`SessionStoreService`] via [`ActorContext`] data injection at
/// startup. The event carries all session data — the actor does not access
/// `AppState`.
pub struct SessionPersistenceActor {
    /// The session store service for writing session snapshots.
    store: Option<SessionStoreService>,
}

impl Actor for SessionPersistenceActor {
    type Message = SessionPersistenceDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<SessionSaveRequested>();
        ctx.subscribe_event::<SessionLoadRequested>();
        let store = ctx.take_data::<SessionStoreService>();
        Self { store }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => self.handle_event(&event, ctx),
            ActorEnvelope::System(SystemMessage::ApplicationReady) => {
                ctx.announce_started();
            }
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Command(_) | ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
}

impl SessionPersistenceActor {
    /// Processes a bus event, saving session data on `SessionSaveRequested`
    /// and loading session data on `SessionLoadRequested`.
    fn handle_event(&mut self, event: &Event, ctx: &ActorContext) {
        match event {
            Event::SessionSaveRequested { payload } => self.on_save_requested(payload),
            Event::SessionLoadRequested { payload } => self.on_load_requested(payload, ctx),
            _ => {}
        }
    }

    /// Constructs a [`PersistedSession`] from the event payload and saves it.
    ///
    /// Errors are logged as warnings — persistence failure must not break
    /// the user experience.
    fn on_save_requested(&mut self, evt: &SessionSaveRequested) {
        let Some(store) = &self.store else {
            tracing::warn!("session persistence actor has no store — dropping save request");
            return;
        };

        let persisted = PersistedSession {
            session_id: evt.session_id.clone(),
            title: evt.title.clone(),
            updated_at: Timestamp::now(),
            history: evt.history.clone(),
            active_strategy: evt.active_strategy.clone(),
            blobs: evt.blobs.clone(),
        };

        if let Err(e) = store.save(&persisted) {
            tracing::warn!(
                session_id = ?evt.session_id,
                err = ?e,
                "failed to persist session"
            );
        }
    }

    /// Loads a full session from disk and sends back a `SessionLoadCompleted` command.
    ///
    /// Seeks to the byte offset from the event payload, reads the session data,
    /// and sends the result back via `send_command`. Errors are logged and produce
    /// an empty `SessionLoadCompleted` to clear the loading state.
    fn on_load_requested(&mut self, evt: &SessionLoadRequested, ctx: &ActorContext) {
        use nullslop_protocol::session::SessionLoadCompleted;

        let Some(store) = &self.store else {
            tracing::warn!("session persistence actor has no store — dropping load request");
            return;
        };

        match store.load_full(evt.byte_offset) {
            Ok(Some(persisted)) => {
                let _ = ctx.send_command(nullslop_protocol::Command::SessionLoadCompleted {
                    payload: SessionLoadCompleted {
                        session_id: persisted.session_id,
                        title: persisted.title,
                        history: persisted.history,
                        active_strategy: persisted.active_strategy,
                        blobs: persisted.blobs,
                    },
                });
            }
            Ok(None) => {
                tracing::warn!(
                    byte_offset = evt.byte_offset,
                    "session load returned None at offset"
                );
                let _ = ctx.send_command(nullslop_protocol::Command::SessionLoadCompleted {
                    payload: SessionLoadCompleted {
                        session_id: evt.session_id.clone(),
                        title: String::new(),
                        history: vec![],
                        active_strategy: PromptStrategyId::passthrough(),
                        blobs: std::collections::HashMap::new(),
                    },
                });
            }
            Err(e) => {
                tracing::warn!(err = ?e, "failed to load session");
                let _ = ctx.send_command(nullslop_protocol::Command::SessionLoadCompleted {
                    payload: SessionLoadCompleted {
                        session_id: evt.session_id.clone(),
                        title: String::new(),
                        history: vec![],
                        active_strategy: PromptStrategyId::passthrough(),
                        blobs: std::collections::HashMap::new(),
                    },
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    #[expect(
        clippy::unused_trait_names,
        reason = "Actor trait needed for activate() method resolution"
    )]
    use nullslop_actor::Actor;
    use nullslop_actor::{ActorContext, ActorEnvelope, MessageSink};
    use nullslop_protocol::{ChatEntry, Event, PromptStrategyId, SessionId, SessionSaveRequested};
    use nullslop_session::{JsonlSessionStore, SessionStoreService};
    use tempfile::TempDir;

    use super::SessionPersistenceActor;

    /// A recording message sink for testing.
    #[derive(Debug)]
    struct RecordingSink {
        events: std::sync::Mutex<Vec<Event>>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                events: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl MessageSink for RecordingSink {
        fn send_command(&self, _command: nullslop_protocol::Command) -> nullslop_actor::SendResult {
            Ok(())
        }

        #[expect(clippy::unwrap_in_result, reason = "test code")]
        fn send_event(&self, event: Event) -> nullslop_actor::SendResult {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    fn test_context(sink: Arc<RecordingSink>) -> ActorContext {
        ActorContext::new("session-persistence", sink as Arc<dyn MessageSink>)
    }

    fn make_store() -> (TempDir, SessionStoreService) {
        let dir = TempDir::new().expect("temp dir");
        let store = JsonlSessionStore::new_in(dir.path().to_path_buf());
        let service = SessionStoreService::new(Arc::new(store));
        (dir, service)
    }

    fn make_save_event(session_id: &SessionId, title: &str) -> Event {
        Event::SessionSaveRequested {
            payload: SessionSaveRequested {
                session_id: session_id.clone(),
                title: title.to_owned(),
                history: vec![ChatEntry::user("hello"), ChatEntry::assistant("world")],
                active_strategy: PromptStrategyId::passthrough(),
                blobs: HashMap::new(),
            },
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn save_event_creates_summary() {
        // Given a SessionPersistenceActor with a JsonlSessionStore in a temp directory.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When a SessionSaveRequested event is sent to the actor.
        let session_id = SessionId::new();
        let event = make_save_event(&session_id, "Test Session");
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then load_summaries returns the session.
        let summaries = store_service.load_summaries().expect("load_summaries");
        assert_eq!(summaries.len(), 1);
        let (id, summary, _offset) = &summaries[0];
        assert_eq!(id, &session_id);
        assert_eq!(summary.title, "Test Session");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn save_event_preserves_full_data() {
        // Given a SessionPersistenceActor with a JsonlSessionStore in a temp directory.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When a SessionSaveRequested event is sent to the actor.
        let session_id = SessionId::new();
        let event = make_save_event(&session_id, "Test Session");
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // And load_full returns matching data.
        let summaries = store_service.load_summaries().expect("load_summaries");
        let full = store_service
            .load_full(summaries[0].2)
            .expect("load_full")
            .expect("should have session");
        assert_eq!(full.session_id, session_id);
        assert_eq!(full.history.len(), 2);
        assert_eq!(full.title, "Test Session");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_persistence_actor_saves_with_blobs() {
        // Given a SessionPersistenceActor with a store.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When a SessionSaveRequested event with blobs is sent.
        let session_id = SessionId::new();
        let event = Event::SessionSaveRequested {
            payload: SessionSaveRequested {
                session_id: session_id.clone(),
                title: "Blob Session".to_owned(),
                history: vec![ChatEntry::user("test")],
                active_strategy: PromptStrategyId::sliding_window(),
                blobs: HashMap::from([(
                    "strategy_state".to_owned(),
                    serde_json::json!({"compaction_count": 5}),
                )]),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then load_full returns the session with blobs preserved.
        let summaries = store_service.load_summaries().expect("load_summaries");
        let full = store_service
            .load_full(summaries[0].2)
            .expect("load_full")
            .expect("should have session");
        assert_eq!(full.active_strategy, PromptStrategyId::sliding_window());
        assert_eq!(full.blobs["strategy_state"]["compaction_count"], 5);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_persistence_actor_ignores_other_events() {
        // Given a SessionPersistenceActor with a store.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When a non-SessionSaveRequested event is sent.
        let event = Event::WorkflowCompleted;
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then no session is saved.
        let summaries = store_service.load_summaries().expect("load_summaries");
        assert!(summaries.is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_persistence_actor_handles_missing_store_gracefully() {
        // Given a SessionPersistenceActor WITHOUT a store injected.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When a SessionSaveRequested event is sent.
        let session_id = SessionId::new();
        let event = make_save_event(&session_id, "No Store");

        // Then the actor does not panic.
        actor.handle(ActorEnvelope::Event(event), &ctx).await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_persistence_actor_saves_multiple_sessions() {
        // Given a SessionPersistenceActor with a store.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When saving two different sessions.
        let id_a = SessionId::new();
        let id_b = SessionId::new();
        actor
            .handle(ActorEnvelope::Event(make_save_event(&id_a, "A")), &ctx)
            .await;
        actor
            .handle(ActorEnvelope::Event(make_save_event(&id_b, "B")), &ctx)
            .await;

        // Then both sessions are in the store.
        let summaries = store_service.load_summaries().expect("load_summaries");
        assert_eq!(summaries.len(), 2);
        let titles: Vec<&str> = summaries.iter().map(|(_, s, _)| s.title.as_str()).collect();
        assert!(titles.contains(&"A"));
        assert!(titles.contains(&"B"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_save_creates_summary() {
        // Given a SessionPersistenceActor with a store.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When saving a session with history and blobs.
        let session_id = SessionId::new();
        let event = Event::SessionSaveRequested {
            payload: SessionSaveRequested {
                session_id: session_id.clone(),
                title: "Round Trip".to_owned(),
                history: vec![
                    ChatEntry::user("first message"),
                    ChatEntry::assistant("first response"),
                    ChatEntry::user("second message"),
                ],
                active_strategy: PromptStrategyId::passthrough(),
                blobs: HashMap::from([(
                    "workflow_state".to_owned(),
                    serde_json::json!({"active_step": "step-1"}),
                )]),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then loading summaries shows the saved session.
        let summaries = store_service.load_summaries().expect("load_summaries");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].1.title, "Round Trip");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_load_restores_full_data() {
        // Given a SessionPersistenceActor with a store.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When saving a session with history and blobs.
        let session_id = SessionId::new();
        let event = Event::SessionSaveRequested {
            payload: SessionSaveRequested {
                session_id: session_id.clone(),
                title: "Round Trip".to_owned(),
                history: vec![
                    ChatEntry::user("first message"),
                    ChatEntry::assistant("first response"),
                    ChatEntry::user("second message"),
                ],
                active_strategy: PromptStrategyId::passthrough(),
                blobs: HashMap::from([(
                    "workflow_state".to_owned(),
                    serde_json::json!({"active_step": "step-1"}),
                )]),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then loading the full session restores all data.
        let summaries = store_service.load_summaries().expect("load_summaries");
        let full = store_service
            .load_full(summaries[0].2)
            .expect("load_full")
            .expect("should have session");

        assert_eq!(full.session_id, session_id);
        assert_eq!(full.title, "Round Trip");
        assert_eq!(full.history.len(), 3);
        assert_eq!(full.blobs["workflow_state"]["active_step"], "step-1");
    }
}
