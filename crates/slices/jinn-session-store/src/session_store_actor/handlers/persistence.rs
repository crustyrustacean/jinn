//! Session snapshot persistence.

use jinn_core_types::SessionId;
use jinn_core_types::WorkingInterval;
use jinn_session_store_msg::PersistSession;

use crate::session_store_actor::SessionStoreActor;
use crate::working_time;

impl SessionStoreActor {
    /// Saves a coherent snapshot of the current session to the store.
    ///
    /// The write lock is held only for the cheap timestamp mutation. The
    /// follow-up read lock captures durable state before store I/O runs.
    pub(crate) async fn save_active_session(&self, session_id: &SessionId) {
        let services = self.services.clone();
        let state = self.state.clone();
        let requested_id = session_id.clone();
        let logged_id = session_id.clone();

        // Read the working intervals out of the work-time cell before the
        // blocking capture, so a running turn's open interval is written as
        // open and closes on load rather than being frozen at save time.
        let working = self.working_intervals(session_id);
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

        let Some(mut snapshot) = snapshot else { return };

        // Stamped here rather than read off the session core: the intervals
        // live in the work-time slice's cell, and the core deliberately does
        // not carry them, so there is exactly one copy and one writer.
        snapshot.metadata.working_intervals = working;

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

    /// This session's recorded working intervals, read from the work-time cell.
    ///
    /// A read only. The monitor is the single writer, and this is one of the
    /// two points where a copy is stamped for durable storage.
    pub(crate) fn working_intervals(&self, session_id: &SessionId) -> Vec<WorkingInterval> {
        working_time::working_intervals(&self.services, session_id)
    }
}
