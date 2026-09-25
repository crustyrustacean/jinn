//! Session snapshot persistence.

use jinn_core_types::SessionId;
use jinn_session_store_msg::PersistSession;

use crate::session_store_actor::SessionStoreActor;

impl SessionStoreActor {
    /// Saves a coherent snapshot of the current session to the store.
    ///
    /// The write lock is held only for the cheap timestamp mutation. The
    /// follow-up read lock captures durable state before store I/O runs.
    pub(crate) async fn save_active_session(&self, session_id: &SessionId) {
        let services = self.services.clone();
        let state = self.state.clone();
        let cap = self.session_cap;
        let requested_id = session_id.clone();
        let logged_id = session_id.clone();

        let snapshot = tokio::task::spawn_blocking(move || {
            {
                state.with_session(|view| {
                    if let Some(session) = view.session.map().get_mut(&requested_id) {
                        session.touch();
                    }
                });
            }
            let state = state.read();
            state
                .session
                .get(&requested_id)
                .filter(|session| session.is_persistable())
                .map(jinn_session_state::ChatSessionState::capture_snapshot)
        })
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(?error, "spawn_blocking panicked during session save");
            None
        });

        let Some(snapshot) = snapshot else { return };

        if let Err(error) = services.session_store.save(&snapshot).await {
            tracing::warn!(
                session_id = ?logged_id,
                ?error,
                "failed to persist session"
            );
        }
    }

    /// Persists the requested session immediately.
    pub(crate) async fn handle_persist_session(&self, payload: &PersistSession) {
        self.save_active_session(&payload.session_id).await;
    }
}
