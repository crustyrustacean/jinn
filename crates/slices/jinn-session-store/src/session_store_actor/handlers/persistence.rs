//! Session snapshot persistence.

use jinn_domain::protocol::SessionId;
use jinn_session_store_msg::PersistSession;

use crate::session_store_actor::SessionStoreActor;

impl SessionStoreActor {
    /// Saves the current state of a session to the store.
    ///
    /// The write lock is held only for the cheap timestamp mutation. The
    /// potentially large history clone runs after releasing it, then store I/O
    /// runs without any state lock held.
    pub(crate) async fn save_active_session(&self, session_id: &SessionId) {
        let services = self.services.clone();
        let state = self.state.clone();
        let cap = self.session_cap;
        let requested_id = session_id.clone();
        let logged_id = session_id.clone();

        let session = tokio::task::spawn_blocking(move || {
            {
                state.with_session(&cap, |view| {
                    if let Some(session) = view.session.map().get_mut(&requested_id) {
                        session.touch();
                    }
                });
            }
            state.read().session.get(&requested_id).cloned()
        })
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(?error, "spawn_blocking panicked during session save");
            None
        });

        let Some(session) = session else { return };
        if !session.is_persistable() {
            return;
        }

        if let Err(error) = services.session_store.save(&session).await {
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
