//! Session working-directory command handling.

use jinn_kernel::common::actor_deps::BusPublish;
use jinn_session_lifecycle_msg::{SessionCwdChanged, SetSessionCwd};

use crate::session_lifecycle_actor::SessionLifecycleActor;

impl SessionLifecycleActor {
    pub(in crate::session_lifecycle_actor) async fn handle_set_session_cwd(
        &self,
        payload: &SetSessionCwd,
    ) {
        self.state.with_session(|view| {
            if let Some(session) = view.session.map().get_mut(&payload.session_id) {
                session.set_cwd(payload.cwd.clone());
            }
        });
        self.publish(SessionCwdChanged {
            session_id: payload.session_id.clone(),
            cwd: payload.cwd.clone(),
        })
        .await;
    }

    pub(in crate::session_lifecycle_actor) fn handle_cancel_lifecycle_command(
        &mut self,
        payload: &jinn_session_lifecycle_msg::CancelLifecycleCommand,
    ) {
        // Only the named session's command: another session's in-flight
        // child is untouched.
        if let Some(handle) = self.take_in_flight(&payload.session_id) {
            jinn_kernel::common::process_kill::kill_process_group_by_pid(handle.pid);
            handle.abort_handle.abort();
        }
    }
}
