//! Session working-directory command handling.

use jinn_domain::common::actor_deps::BusPublish;
use jinn_session_lifecycle_msg::{SessionCwdChanged, SetSessionCwd};

use crate::session_lifecycle_actor::SessionLifecycleActor;

impl SessionLifecycleActor {
    pub(in crate::session_lifecycle_actor) async fn handle_set_session_cwd(
        &self,
        payload: &SetSessionCwd,
    ) {
        self.state.with_session(&self.session_cap, |view| {
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
        _payload: &jinn_session_lifecycle_msg::CancelLifecycleCommand,
    ) {
        if let Some(handle) = self.lifecycle_child.take() {
            jinn_domain::common::process_kill::kill_process_group_by_pid(handle.pid);
            handle.abort_handle.abort();
        }
    }
}
