//! Session snapshot persistence.
//!
//! The stamped-save flow itself lives in the kernel
//! ([`jinn_kernel::common::session_persistence`]) — the one flow every
//! durable write routes through, shared with the session-turn actor. This
//! module is the store actor's delegations into it.

use jinn_core_types::SessionId;
use jinn_core_types::WorkingInterval;
use jinn_session_store_msg::PersistSession;

use crate::session_store_actor::SessionStoreActor;

impl SessionStoreActor {
    /// Saves a coherent snapshot of the current session to the store,
    /// stamped with the work-time cell's current record.
    pub(crate) async fn save_active_session(&self, session_id: &SessionId) {
        self.save_active_session_stamping(session_id, None).await;
    }

    /// Saves the session, stamping `override_intervals` onto the snapshot in
    /// place of the work-time cell's current record.
    ///
    /// The one caller is the load path's reactivation write — see the kernel
    /// flow's doc for why the override is load-bearing.
    pub(crate) async fn save_active_session_stamping(
        &self,
        session_id: &SessionId,
        override_intervals: Option<Vec<WorkingInterval>>,
    ) {
        jinn_kernel::common::session_persistence::save_session_stamped(
            &self.services,
            &self.state,
            session_id,
            override_intervals,
        )
        .await;
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
        jinn_kernel::common::session_persistence::working_intervals(&self.services, session_id)
    }
}
